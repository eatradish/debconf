use enumflags2::{BitFlags, bitflags};
use nom::{
    IResult, Parser,
    branch::alt,
    bytes::{complete::tag, take_until},
    combinator::{map, rest},
    sequence::preceded,
};
use std::fmt;

#[bitflags]
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum Capability {
    /// The frontend supports multi-select/checkbox interaction.
    Multiselect = 0b0001,
    /// The frontend supports escaping special text containing newlines as `\n` during transmission.
    Escape = 0b0010,
    /// The frontend supports backing up to the previous question via a backup mechanism.
    Backup = 0b0100,
    /// The frontend supports receiving and displaying progress bar updates from the backend.
    Progress = 0b1000,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescriptionContent {
    Type(String),
    Short(String),
    Extended(String),
    Unknown(String),
}

#[derive(Debug, PartialEq, Clone)]
pub enum DebconfCommand {
    /// Handshake command to declare capabilities, carrying the raw capabilities string supported by the backend.
    Capb(String),
    /// Sets the title text of the current configuration context.
    Title(String),
    /// The core interactive description/prompt text (with status suffixes automatically stripped and physical newlines restored).
    Description {
        question: String,
        content: DescriptionContent,
    },
    /// A comma-separated list of candidate choices sent by the backend for single-choice or multi-choice scenarios.
    Choices(Vec<String>),
    /// Input readiness notification, containing the priority of the question and its unique identifier.
    Input { priority: String, question: String },
    /// The core trigger command: requests the frontend to immediately render the UI and block until the user makes a decision.
    Go,
    /// Sent by the backend to request the final answer decided by the user for a specific configuration item.
    Get(String),
    /// The farewell signal indicating that the configuration transaction has ended normally.
    Goodbye,
    /// Unrecognized or insignificant commands used for debugging.
    Unknown,
}

#[derive(Debug, PartialEq, Clone)]
pub enum DebconfResponse {
    CapbSuccess(BitFlags<Capability>),
    Ok,
    Answer(String),
    Error { code: u32, message: String },
}

impl fmt::Display for DebconfResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DebconfResponse::CapbSuccess(caps) => {
                write!(f, "0 capb")?;
                if caps.contains(Capability::Multiselect) {
                    write!(f, " multiselect")?;
                }
                if caps.contains(Capability::Escape) {
                    write!(f, " escape")?;
                }
                if caps.contains(Capability::Backup) {
                    write!(f, " backup")?;
                }
                if caps.contains(Capability::Progress) {
                    write!(f, " progress")?;
                }
                write!(f, "\n")
            }
            DebconfResponse::Ok => write!(f, "0 ok\n"),
            DebconfResponse::Answer(ans) => {
                let escaped = ans.replace('\n', "\\n");
                write!(f, "0 {}\n", escaped)
            }
            DebconfResponse::Error { code, message } => {
                write!(f, "{} {}\n", code, message)
            }
        }
    }
}

pub struct DebconfWriter<W: std::io::Write> {
    inner: W,
}

impl<W: std::io::Write> DebconfWriter<W> {
    pub fn new(inner: W) -> Self {
        Self { inner }
    }

    pub fn send(&mut self, response: &DebconfResponse) -> std::io::Result<()> {
        write!(self.inner, "{}", response)?;
        self.inner.flush()
    }
}

fn parse_capb(input: &str) -> IResult<&str, DebconfCommand> {
    map(preceded(tag("CAPB "), rest), |caps: &str| {
        DebconfCommand::Capb(caps.to_string())
    })
    .parse(input)
}

fn parse_title(input: &str) -> IResult<&str, DebconfCommand> {
    map(preceded(tag("TITLE "), rest), |t: &str| {
        DebconfCommand::Title(t.to_string())
    })
    .parse(input)
}

fn parse_and_unescape(mut input: &str) -> String {
    let mut result = String::new();
    
    while !input.is_empty() {
        if let Some(rest) = input.strip_prefix(r#"\n"#).or_else(|| input.strip_prefix("\\n")) {
            result.push('\n');
            input = rest;
        } else if let Some(rest) = input.strip_prefix('\\') {
            result.push('\\');
            if !rest.is_empty() {
                let (ch, next) = rest.split_at(1);
                result.push_str(ch);
                input = next;
            } else {
                input = rest;
            }
        } else {
            if let Some(pos) = input.find('\\') {
                let (left, right) = input.split_at(pos);
                result.push_str(left);
                input = right;
            } else {
                result.push_str(input);
                break;
            }
        }
    }

    result
}

pub fn parse_data(input: &str) -> IResult<&str, DebconfCommand> {
    let (input, _) = alt((tag("DATA "), tag("METAGET "))).parse(input)?;

    let (input, question) = take_until(" ").parse(input)?;
    let question = question.trim().to_string();

    let (input, _) = tag(" ").parse(input)?;

    let clean_text = match input.rsplit_once(": ") {
        Some((text_part, _)) => text_part.trim(),
        None => input.trim(),
    };

    let content_result: IResult<&str, DescriptionContent> = alt((
        preceded(tag("type "), rest).map(|t: &str| DescriptionContent::Type(t.trim().to_string())),
        preceded(tag("description "), rest).map(|s: &str| {
            let unescaped = parse_and_unescape(s.trim());
            DescriptionContent::Short(unescaped)
        }),
        preceded(tag("extended_description "), rest).map(|e: &str| {
            let unescaped = parse_and_unescape(e.trim());
            DescriptionContent::Extended(unescaped)
        }),
        rest.map(|u: &str| {
            let unescaped = parse_and_unescape(u.trim());
            DescriptionContent::Unknown(unescaped)
        }),
    ))
    .parse(clean_text);

    let (_, content) = content_result.unwrap();

    Ok(("", DebconfCommand::Description { question, content }))
}

fn parse_choices(input: &str) -> IResult<&str, DebconfCommand> {
    let (rest, _) = tag("CHOICES ")(input)?;
    let choices = rest.split(", ").map(|s| s.to_string()).collect();
    Ok(("", DebconfCommand::Choices(choices)))
}

fn parse_input(input: &str) -> IResult<&str, DebconfCommand> {
    let (rest, _) = tag("INPUT ")(input)?;
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() >= 2 {
        Ok((
            "",
            DebconfCommand::Input {
                priority: parts[0].to_string(),
                question: parts[1].to_string(),
            },
        ))
    } else {
        Ok((
            "",
            DebconfCommand::Input {
                priority: "critical".to_string(),
                question: rest.to_string(),
            },
        ))
    }
}

fn parse_get(input: &str) -> IResult<&str, DebconfCommand> {
    map(preceded(tag("GET "), rest), |arg: &str| {
        DebconfCommand::Get(arg.to_string())
    })
    .parse(input)
}

pub fn parse_line(input: &str) -> DebconfCommand {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return DebconfCommand::Unknown;
    }
    if trimmed == "GO" {
        return DebconfCommand::Go;
    }
    if trimmed == "GOODBYE" {
        return DebconfCommand::Goodbye;
    }

    let result: IResult<&str, DebconfCommand> = nom::branch::alt((
        parse_capb,
        parse_title,
        parse_data,
        parse_choices,
        parse_input,
        parse_get,
    ))
    .parse(trimmed);

    match result {
        Ok((_, cmd)) => cmd,
        Err(_) => DebconfCommand::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_commands() {
        assert_eq!(parse_line("GO"), DebconfCommand::Go);
        assert_eq!(parse_line("GOODBYE"), DebconfCommand::Goodbye);

        let raw_data_line =
            "DATA my-app/host_prompt extended_description please enter host\\nname: yes";
        if let DebconfCommand::Description { question, content } = parse_line(raw_data_line) {
            assert_eq!(question, "my-app/host_prompt");
            assert_eq!(
                content,
                DescriptionContent::Extended("please enter host\nname".to_string())
            );
        } else {
            panic!("DATA extended_description parse failed");
        }

        let raw_note_line = "DATA mise/configuration_hints type note";
        if let DebconfCommand::Description { question, content } = parse_line(raw_note_line) {
            assert_eq!(question, "mise/configuration_hints");
            assert_eq!(content, DescriptionContent::Type("note".to_string()));
        } else {
            panic!("DATA type note parse failed");
        }
    }

    #[test]
    fn test_response_serialization() {
        assert_eq!(DebconfResponse::Ok.to_string(), "0 ok\n");

        let caps = Capability::Multiselect | Capability::Escape;
        let response = DebconfResponse::CapbSuccess(caps);
        assert_eq!(response.to_string(), "0 capb multiselect escape\n");
    }
}
