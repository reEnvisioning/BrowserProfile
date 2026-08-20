use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Browser {
    Firefox,
    Librewolf,
}
impl Browser {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "firefox" => Some(Self::Firefox),
            "librewolf" => Some(Self::Librewolf),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Firefox => "firefox",
            Self::Librewolf => "librewolf",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    List {
        browser: Option<Browser>,
        all: bool,
    },
    Create {
        browser: Browser,
        name: String,
        default: bool,
    },
    Apply {
        browser: Browser,
        name: String,
        backup: bool,
    },
    DefaultGet {
        browser: Browser,
    },
    DefaultSet {
        browser: Browser,
        name: String,
    },
    Remove {
        browser: Browser,
        name: String,
        yes: bool,
    },
    Launch {
        browser: Browser,
        name: String,
        private: bool,
        args: Vec<String>,
    },
}
impl Command {
    pub fn browser(&self) -> Option<Browser> {
        match self {
            Self::List { browser, .. } => *browser,
            Self::Create { browser, .. }
            | Self::Apply { browser, .. }
            | Self::DefaultGet { browser }
            | Self::DefaultSet { browser, .. }
            | Self::Remove { browser, .. }
            | Self::Launch { browser, .. } => Some(*browser),
        }
    }
}

pub fn usage() {
    eprintln!("usage: bp <list|create|apply|default get|default set|remove|launch> ...");
}

fn positional_args(args: &[String]) -> Vec<&str> {
    let end = args.iter().position(|x| x == "--").unwrap_or(args.len());
    let mut out = Vec::new();
    let mut i = 0;
    while i < end {
        if matches!(args[i].as_str(), "--browser" | "--template") {
            i += 2;
        } else if args[i].starts_with('-') {
            i += 1;
        } else {
            out.push(args[i].as_str());
            i += 1;
        }
    }
    out
}

fn option(args: &[String], key: &str, default: Browser) -> io::Result<Browser> {
    let mut found = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == key {
            let value = args.get(i + 1).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "missing option value")
            })?;
            if value.starts_with('-') {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "missing option value",
                ));
            }
            if found.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "duplicate option",
                ));
            }
            found = Some(Browser::parse(value).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "browser must be firefox or librewolf",
                )
            })?);
            i += 2;
        } else {
            i += 1;
        }
    }
    Ok(found.unwrap_or(default))
}

fn validate(command: &str, args: &[String]) -> io::Result<()> {
    let separator = args.iter().position(|x| x == "--");
    if separator.is_some() && command != "launch" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "-- is only valid for launch",
        ));
    }
    let end = separator.unwrap_or(args.len());
    let mut positional = 0;
    let mut positional_values = Vec::new();
    let mut i = 0;
    while i < end {
        let x = &args[i];
        if x == "--browser" {
            let value = args
                .get(i + 1)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing browser"))?;
            if Browser::parse(value).is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "browser must be firefox or librewolf",
                ));
            }
            i += 2;
            continue;
        }
        if x == "--template" {
            if command != "create" && command != "apply" {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "unknown option",
                ));
            }
            let value = args
                .get(i + 1)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing template"))?;
            if value != "strict" {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "template must be strict",
                ));
            }
            i += 2;
            continue;
        }
        let allowed = match command {
            "list" => ["--all"].contains(&x.as_str()),
            "create" => ["--default"].contains(&x.as_str()),
            "apply" => ["--backup"].contains(&x.as_str()),
            "remove" => ["--yes"].contains(&x.as_str()),
            "launch" => ["--private-window"].contains(&x.as_str()),
            "default" => false,
            _ => false,
        };
        if allowed {
            i += 1;
            continue;
        }
        if x.starts_with('-') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unknown option",
            ));
        }
        positional += 1;
        positional_values.push(x.as_str());
        i += 1;
    }
    for flag in [
        "--all",
        "--default",
        "--backup",
        "--yes",
        "--private-window",
        "--template",
    ] {
        if args[..end].iter().filter(|x| x.as_str() == flag).count() > 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "duplicate option",
            ));
        }
    }
    if command == "default" {
        let valid = positional_values.as_slice() == ["get"]
            || (positional_values.len() == 2 && positional_values[0] == "set");
        if !valid {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "use default get or default set <name>",
            ));
        }
    } else if (command == "list" && positional != 0) || (command != "list" && positional != 1) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid arguments",
        ));
    }
    Ok(())
}

pub fn parse(mut args: Vec<String>) -> io::Result<Command> {
    if args.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "missing command",
        ));
    }
    let command = args.remove(0);
    validate(&command, &args)?;
    let before_separator = args.iter().position(|x| x == "--").unwrap_or(args.len());
    let explicit_browser = args[..before_separator]
        .iter()
        .any(|arg| arg == "--browser");
    let browser = option(&args[..before_separator], "--browser", Browser::Firefox)?;
    let positional = positional_args(&args);
    match command.as_str() {
        "list" => Ok(Command::List {
            browser: explicit_browser.then_some(browser),
            all: args.iter().any(|x| x == "--all"),
        }),
        "create" => Ok(Command::Create {
            browser,
            name: positional[0].to_owned(),
            default: args.iter().any(|x| x == "--default"),
        }),
        "apply" => Ok(Command::Apply {
            browser,
            name: positional[0].to_owned(),
            backup: args.iter().any(|x| x == "--backup"),
        }),
        "remove" => Ok(Command::Remove {
            browser,
            name: positional[0].to_owned(),
            yes: args.iter().any(|x| x == "--yes"),
        }),
        "launch" => {
            let split = args.iter().position(|x| x == "--");
            Ok(Command::Launch {
                browser,
                name: positional[0].to_owned(),
                private: args[..before_separator]
                    .iter()
                    .any(|x| x == "--private-window"),
                args: split.map(|i| args[i + 1..].to_vec()).unwrap_or_default(),
            })
        }
        "default" => match positional[0] {
            "get" => Ok(Command::DefaultGet { browser }),
            "set" => Ok(Command::DefaultSet {
                browser,
                name: positional[1].to_owned(),
            }),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "use default get or default set",
            )),
        },
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unknown command",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_launch_arguments_after_separator() {
        assert_eq!(
            parse(vec![
                "launch".into(),
                "profile".into(),
                "--private-window".into(),
                "--".into(),
                "https://example.com".into(),
            ])
            .unwrap(),
            Command::Launch {
                browser: Browser::Firefox,
                name: "profile".into(),
                private: true,
                args: vec!["https://example.com".into()],
            }
        );
    }

    #[test]
    fn parses_other_commands() {
        assert_eq!(
            parse(vec![
                "list".into(),
                "--browser".into(),
                "librewolf".into(),
                "--all".into(),
            ])
            .unwrap(),
            Command::List {
                browser: Some(Browser::Librewolf),
                all: true,
            }
        );
        assert_eq!(
            parse(vec!["create".into(), "work".into(), "--default".into()]).unwrap(),
            Command::Create {
                browser: Browser::Firefox,
                name: "work".into(),
                default: true,
            }
        );
        assert_eq!(
            parse(vec!["apply".into(), "work".into(), "--backup".into()]).unwrap(),
            Command::Apply {
                browser: Browser::Firefox,
                name: "work".into(),
                backup: true,
            }
        );
        assert_eq!(
            parse(vec!["default".into(), "get".into()]).unwrap(),
            Command::DefaultGet {
                browser: Browser::Firefox,
            }
        );
        assert_eq!(
            parse(vec!["default".into(), "set".into(), "work".into()]).unwrap(),
            Command::DefaultSet {
                browser: Browser::Firefox,
                name: "work".into(),
            }
        );
        assert_eq!(
            parse(vec!["remove".into(), "work".into(), "--yes".into()]).unwrap(),
            Command::Remove {
                browser: Browser::Firefox,
                name: "work".into(),
                yes: true,
            }
        );
    }

    #[test]
    fn rejects_malformed_options() {
        for args in [
            vec!["list", "profile"],
            vec!["list", "--browser"],
            vec!["list", "--browser", "other"],
            vec!["list", "--browser", "firefox", "--browser", "librewolf"],
            vec!["create", "work", "--default", "--default"],
            vec!["default", "set"],
            vec!["remove", "work", "--", "extra"],
        ] {
            assert!(parse(args.into_iter().map(String::from).collect()).is_err());
        }
    }
}
