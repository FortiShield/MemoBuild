use anyhow::{bail, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocatedInstruction {
    pub instruction: Instruction,
    pub location: SourceLocation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Instruction {
    From {
        image: String,
        options: Vec<String>,
    },
    Workdir(String),
    Copy {
        flags: Vec<String>,
        sources: Vec<String>,
        destination: String,
    },
    Run(String),
    Env {
        key: String,
        value: String,
    },
    Cmd(Vec<String>),
    Entrypoint(Vec<String>),
    Unsupported {
        keyword: String,
        args: String,
    },
}

pub fn parse_dockerfile(content: &str) -> Result<Vec<LocatedInstruction>> {
    let mut instructions = Vec::new();

    for (line_index, raw_line) in content.lines().enumerate() {
        let line_number = line_index + 1;
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') {
            let directive = trimmed.to_ascii_lowercase();
            if directive.starts_with("# syntax=") || directive.starts_with("# escape=") {
                bail!(
                    "Dockerfile line {line_number}: parser directives are unsupported; \
                     MemoBuild uses its built-in strict syntax"
                );
            }
            continue;
        }
        if trimmed.ends_with('\\') {
            bail!(
                "Dockerfile line {line_number}: line continuations are unsupported; \
                 put each instruction on one physical line"
            );
        }

        let column = raw_line.len() - raw_line.trim_start().len() + 1;
        let (keyword, args) = split_keyword(trimmed);
        let keyword = keyword.to_ascii_uppercase();
        if !is_recognized(&keyword) {
            bail!(
                "Dockerfile line {line_number}: unknown instruction '{keyword}'; \
                 remove it or use an instruction supported by MemoBuild"
            );
        }
        if args.is_empty() {
            bail!("Dockerfile line {line_number}: {keyword} requires arguments");
        }

        let instruction = match keyword.as_str() {
            "FROM" => parse_from(line_number, args)?,
            "WORKDIR" => {
                let tokens = parse_tokens(line_number, &keyword, args)?;
                if tokens.len() != 1 {
                    bail!("Dockerfile line {line_number}: WORKDIR requires exactly one path");
                }
                Instruction::Workdir(tokens[0].clone())
            }
            "COPY" => parse_copy(line_number, args)?,
            "RUN" => {
                validate_optional_json_command(line_number, &keyword, args)?;
                Instruction::Run(args.to_string())
            }
            "ENV" => parse_env(line_number, args)?,
            "CMD" => Instruction::Cmd(parse_json_command(line_number, &keyword, args)?),
            "ENTRYPOINT" => {
                Instruction::Entrypoint(parse_json_command(line_number, &keyword, args)?)
            }
            "ADD" => {
                let tokens = parse_tokens(line_number, &keyword, args)?;
                if tokens.len() < 2 {
                    bail!("Dockerfile line {line_number}: ADD requires a source and destination");
                }
                Instruction::Unsupported {
                    keyword,
                    args: args.to_string(),
                }
            }
            "ARG" => {
                let name = args.split('=').next().unwrap_or_default();
                validate_env_name(line_number, &keyword, name)?;
                Instruction::Unsupported {
                    keyword,
                    args: args.to_string(),
                }
            }
            "USER" | "SHELL" | "LABEL" | "EXPOSE" | "VOLUME" | "HEALTHCHECK" | "STOPSIGNAL"
            | "ONBUILD" | "MAINTAINER" | "GIT" | "RUN_EXTEND" | "COPY_EXTEND" | "HOOK" => {
                validate_known_arguments(line_number, &keyword, args)?;
                Instruction::Unsupported {
                    keyword,
                    args: args.to_string(),
                }
            }
            _ => unreachable!("recognized instruction has a parser branch"),
        };

        instructions.push(LocatedInstruction {
            instruction,
            location: SourceLocation {
                line: line_number,
                column,
            },
        });
    }

    fn is_recognized(keyword: &str) -> bool {
        matches!(
            keyword,
            "FROM"
                | "WORKDIR"
                | "COPY"
                | "RUN"
                | "ENV"
                | "CMD"
                | "ENTRYPOINT"
                | "ADD"
                | "ARG"
                | "USER"
                | "SHELL"
                | "LABEL"
                | "EXPOSE"
                | "VOLUME"
                | "HEALTHCHECK"
                | "STOPSIGNAL"
                | "ONBUILD"
                | "MAINTAINER"
                | "GIT"
                | "RUN_EXTEND"
                | "COPY_EXTEND"
                | "HOOK"
        )
    }

    Ok(instructions)
}

fn split_keyword(line: &str) -> (&str, &str) {
    match line.find(char::is_whitespace) {
        Some(index) => (&line[..index], line[index..].trim()),
        None => (line, ""),
    }
}

fn parse_from(line: usize, args: &str) -> Result<Instruction> {
    let tokens = parse_tokens(line, "FROM", args)?;
    if tokens.is_empty() {
        bail!("Dockerfile line {line}: FROM requires a base image");
    }
    Ok(Instruction::From {
        image: tokens[0].clone(),
        options: tokens[1..].to_vec(),
    })
}

fn parse_copy(line: usize, args: &str) -> Result<Instruction> {
    if args.starts_with('[') {
        bail!(
            "Dockerfile line {line}: JSON-form COPY is unsupported; use one source and one destination"
        );
    }
    let tokens = parse_tokens(line, "COPY", args)?;
    let mut flags = Vec::new();
    let mut paths = Vec::new();
    let mut index = 0;
    while index < tokens.len() && tokens[index].starts_with("--") {
        if tokens[index] == "--from" {
            let value = tokens.get(index + 1).ok_or_else(|| {
                anyhow::anyhow!("Dockerfile line {line}: COPY --from requires a stage reference")
            })?;
            flags.push(format!("--from={value}"));
            index += 2;
        } else {
            flags.push(tokens[index].clone());
            index += 1;
        }
    }
    paths.extend(tokens[index..].iter().cloned());
    if paths.len() < 2 {
        bail!("Dockerfile line {line}: COPY requires at least one source and a destination");
    }
    let destination = paths.pop().expect("two or more paths were validated");
    Ok(Instruction::Copy {
        flags,
        sources: paths,
        destination,
    })
}

fn parse_env(line: usize, args: &str) -> Result<Instruction> {
    let (key, value) = if let Some((key, value)) = args.split_once('=') {
        if key.chars().any(char::is_whitespace) {
            bail!("Dockerfile line {line}: ENV key=value form must contain one key");
        }
        (key, value)
    } else {
        let (key, value) = args.split_once(char::is_whitespace).ok_or_else(|| {
            anyhow::anyhow!("Dockerfile line {line}: ENV requires a key and value")
        })?;
        (key, value.trim())
    };
    validate_env_name(line, "ENV", key)?;
    if value.is_empty() {
        bail!("Dockerfile line {line}: ENV requires a nonempty value");
    }
    Ok(Instruction::Env {
        key: key.to_string(),
        value: value.to_string(),
    })
}

fn validate_env_name(line: usize, keyword: &str, name: &str) -> Result<()> {
    let mut chars = name.chars();
    let valid = chars
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric());
    if !valid {
        bail!("Dockerfile line {line}: {keyword} has invalid variable name '{name}'");
    }
    Ok(())
}

fn parse_json_command(line: usize, keyword: &str, args: &str) -> Result<Vec<String>> {
    if !args.starts_with('[') {
        bail!(
            "Dockerfile line {line}: shell-form {keyword} is unsupported; use a JSON string array"
        );
    }
    let command: Vec<String> = serde_json::from_str(args).map_err(|error| {
        anyhow::anyhow!("Dockerfile line {line}: malformed JSON form for {keyword}: {error}")
    })?;
    if command.is_empty() || command[0].is_empty() {
        bail!("Dockerfile line {line}: JSON-form {keyword} requires a nonempty executable");
    }
    Ok(command)
}

fn validate_optional_json_command(line: usize, keyword: &str, args: &str) -> Result<()> {
    if args.starts_with('[') {
        let command: Vec<String> = serde_json::from_str(args).map_err(|error| {
            anyhow::anyhow!("Dockerfile line {line}: malformed JSON form for {keyword}: {error}")
        })?;
        if command.is_empty() || command[0].is_empty() {
            bail!("Dockerfile line {line}: JSON-form {keyword} requires a nonempty executable");
        }
    }
    Ok(())
}

fn validate_known_arguments(line: usize, keyword: &str, args: &str) -> Result<()> {
    match keyword {
        "SHELL" => {
            let shell: Vec<String> = serde_json::from_str(args).map_err(|error| {
                anyhow::anyhow!("Dockerfile line {line}: malformed JSON form for SHELL: {error}")
            })?;
            if shell.is_empty() || shell.iter().any(String::is_empty) {
                bail!("Dockerfile line {line}: SHELL requires a nonempty string array");
            }
        }
        "EXPOSE" => {
            for value in parse_tokens(line, keyword, args)? {
                let port = value.split('/').next().unwrap_or_default();
                if port.parse::<u16>().is_err() {
                    bail!("Dockerfile line {line}: invalid EXPOSE port '{value}'");
                }
            }
        }
        "COPY_EXTEND" => {
            if parse_tokens(line, keyword, args)?.len() < 2 {
                bail!("Dockerfile line {line}: COPY_EXTEND requires source and destination");
            }
        }
        "HOOK" => {
            if parse_tokens(line, keyword, args)?.is_empty() {
                bail!("Dockerfile line {line}: HOOK requires a hook name");
            }
        }
        "GIT" => {
            if parse_tokens(line, keyword, args)?.is_empty() {
                bail!("Dockerfile line {line}: GIT requires a repository URL");
            }
        }
        "RUN_EXTEND" | "USER" | "LABEL" | "VOLUME" | "HEALTHCHECK" | "STOPSIGNAL" | "ONBUILD"
        | "MAINTAINER" => {}
        _ => {}
    }
    Ok(())
}

fn parse_tokens(line: usize, keyword: &str, args: &str) -> Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut escaped = false;

    for ch in args.chars() {
        if escaped {
            token.push(ch);
            escaped = false;
            continue;
        }
        match (quote, ch) {
            (Some('"'), '\\') => escaped = true,
            (Some(current), ch) if current == ch => quote = None,
            (Some(_), ch) => token.push(ch),
            (None, '\'' | '"') => quote = Some(ch),
            (None, ch) if ch.is_whitespace() => {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
            }
            (None, ch) => token.push(ch),
        }
    }
    if escaped || quote.is_some() {
        bail!("Dockerfile line {line}: malformed quoting in {keyword} arguments");
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    Ok(tokens)
}
