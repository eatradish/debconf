# debconf

A lightweight debconf protocol parser and serializer.

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
debconf = "0.1.0"
dialoguer = "0.12"
```

Below is a basic example of how to drive the parser and writer using standard I/O streams:

```rust
use debconf::{parse_line, DebconfCommand, DebconfResponse, DebconfWriter, Capability, DescriptionContent};
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Select};
use std::io::{BufRead, Write};

pub fn run_debconf_loop<R: BufRead, W: Write>(mut reader: R, raw_writer: W) -> std::io::Result<()> {
    let mut line = String::new();
    let mut tx = DebconfWriter::new(raw_writer);

    // 协议内部状态机上下文缓存
    let mut current_description = String::from("请选择配置项：");
    let mut current_choices = Vec::new();
    let mut last_user_answer = String::new();
    
    // 🔑 从单一的布尔标记升级为强类型的字符串题型跟踪
    let mut current_question_type = String::from("string");

    while reader.read_line(&mut line)? > 0 {
        let cmd = parse_line(&line);

        // 判定是否收到 Goodbye 信号以优雅退出
        let should_break = matches!(cmd, DebconfCommand::Goodbye);

        let response = match cmd {
            DebconfCommand::Capb(_) => {
                Some(DebconfResponse::CapbSuccess(Capability::Multiselect | Capability::Escape))
            }
            DebconfCommand::Title(title) => {
                println!("=== {} ===", title);
                Some(DebconfResponse::Ok)
            }
            DebconfCommand::Description { question: _, content } => {
                match content {
                    DescriptionContent::Type(t) => {
                        current_question_type = t
                    }
                    DescriptionContent::Short(text) | DescriptionContent::Extended(text) | DescriptionContent::Unknown(text) => {
                        current_description = text;
                    }
                }
                Some(DebconfResponse::Ok)
            }
            DebconfCommand::Choices(choices) => {
                current_choices = choices;
                Some(DebconfResponse::Ok)
            }
            DebconfCommand::Input { priority: _, question } => {
                if current_question_type == "string" {
                    if question.contains("boolean") {
                        current_question_type = String::from("boolean");
                    } else if question.contains("select") {
                        current_question_type = String::from("select");
                    } else if question.contains("note") || question.contains("error") {
                        current_question_type = String::from("note");
                    }
                }
                Some(DebconfResponse::Ok)
            }

            DebconfCommand::Go => {
                let theme = ColorfulTheme::default();

                match current_question_type.as_str() {
                    "note" | "error" => {
                        println!("\n{}", current_description);
                        
                        let _ = Input::<String>::with_theme(&theme)
                            .with_prompt("Press [Enter] to continue")
                            .allow_empty(true)
                            .report(false)
                            .interact_text()
                            .unwrap_or_default();

                        last_user_answer = String::new();
                    }
                    "boolean" => {
                        let selection = Confirm::with_theme(&theme)
                            .with_prompt(&current_description)
                            .default(true)
                            .interact()
                            .unwrap_or(true);

                        last_user_answer = selection.to_string();
                    }
                    "select" => {
                        if !current_choices.is_empty() {
                            let selection = Select::with_theme(&theme)
                                .with_prompt(&current_description)
                                .items(&current_choices)
                                .default(0)
                                .interact()
                                .unwrap_or(0);

                            last_user_answer = current_choices[selection].clone();
                        } else {
                            last_user_answer = String::new();
                        }
                    }
                    _ => {
                        let text_input = Input::<String>::with_theme(&theme)
                            .with_prompt(&current_description)
                            .allow_empty(true)
                            .interact_text()
                            .unwrap_or_default();

                        last_user_answer = text_input;
                    }
                }

                current_choices.clear();
                current_question_type = String::from("string");

                Some(DebconfResponse::Ok)
            }

            DebconfCommand::Get(_) => {
                Some(DebconfResponse::Answer(last_user_answer.clone()))
            }

            DebconfCommand::Goodbye => None,
            _ => Some(DebconfResponse::Ok),
        };

        // 统一的数据应答喷回与挂断逻辑
        if let Some(resp) = response {
            if let Err(e) = tx.send(&resp) {
                eprintln!("Failed to send response to debconf backend: {}", e);
                break;
            }
        }

        if should_break {
            break;
        }

        line.clear();
    }

    Ok(())
}
```

## License

This project is licensed under the MIT License.
