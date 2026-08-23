use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Browser {
    Firefox,
    Librewolf,
}
impl Browser {
    pub fn parse(s: &str) -> Option<Self> {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProfileId(pub u64);
impl ProfileId {
    pub fn parse(s: &str) -> io::Result<Self> {
        let Some(number) = s.strip_prefix('@') else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "profile ID must start with @",
            ));
        };
        if number.is_empty()
            || (number.len() > 1 && number.starts_with('0'))
            || !number.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "profile ID must be canonical, such as @0 or @1",
            ));
        }
        number
            .parse()
            .map(Self)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "profile ID is too large"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selector {
    Id(ProfileId),
    Name(String),
}
fn selector(value: &str) -> io::Result<Selector> {
    if value.starts_with('@') || value.starts_with('#') {
        ProfileId::parse(value).map(Selector::Id)
    } else {
        Ok(Selector::Name(value.into()))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    List {
        browser: Option<Browser>,
        all: bool,
    },
    Create {
        id: Option<ProfileId>,
        browser: Option<Browser>,
        name: Option<String>,
        default: Option<bool>,
    },
    Apply {
        browser: Option<Browser>,
        selector: Selector,
        backup: bool,
    },
    DefaultGet {
        browser: Option<Browser>,
    },
    DefaultSet {
        browser: Option<Browser>,
        selector: Selector,
    },
    Remove {
        browser: Option<Browser>,
        selector: Selector,
        yes: bool,
    },
    Launch {
        browser: Option<Browser>,
        selector: Option<Selector>,
        private: bool,
        args: Vec<String>,
    },
}

pub const HELP: &str = "usage: bp <list|create|apply|default get|default set|remove|launch> ...\n\nProfile IDs use shell-safe @0, @1 syntax.\n\nbp create [@ID] [--name NAME] [--browser firefox|librewolf] [--default yes|no]\nbp apply @ID|NAME [--browser firefox|librewolf] [--backup]\nbp default get [--browser firefox|librewolf]\nbp default set @ID|NAME [--browser firefox|librewolf]\nbp remove @ID|NAME [--browser firefox|librewolf] [--yes]\nbp launch [@ID|NAME] [--browser firefox|librewolf] [--private-window] [-- ARGS...]\nbp list [--browser firefox|librewolf] [--all]";

pub fn usage() {
    eprintln!(
        "usage: bp <list|create|apply|default get|default set|remove|launch> ...; IDs use @1; run 'bp help'"
    );
}

fn error(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn take_value(args: &[String], i: &mut usize, option: &str) -> io::Result<String> {
    *i += 1;
    let value = args
        .get(*i)
        .ok_or_else(|| error(&format!("missing value for {option}")))?;
    if value.starts_with('-') {
        return Err(error(&format!("missing value for {option}")));
    }
    Ok(value.clone())
}

pub fn parse(args: Vec<String>) -> io::Result<Command> {
    let (command, rest) = args.split_first().ok_or_else(|| error("missing command"))?;
    if command == "help" && rest.is_empty() {
        return Err(error("help is handled by main"));
    }
    let (action, args) = if command == "default" {
        let (action, args) = rest
            .split_first()
            .ok_or_else(|| error("use default get or default set <profile>"))?;
        (format!("default-{action}"), args)
    } else {
        (command.clone(), rest)
    };
    if !matches!(
        action.as_str(),
        "list" | "create" | "apply" | "default-get" | "default-set" | "remove" | "launch"
    ) {
        return Err(error("unknown command"));
    }

    let split = args.iter().position(|arg| arg == "--");
    if split.is_some() && action != "launch" {
        return Err(error("-- is only valid for launch"));
    }
    let end = split.unwrap_or(args.len());
    let mut browser = None;
    let mut name = None;
    let mut default = None;
    let mut template = false;
    let mut backup = false;
    let mut yes = false;
    let mut private = false;
    let mut all = false;
    let mut positional = Vec::new();
    let mut i = 0;
    while i < end {
        match args[i].as_str() {
            "--browser" => {
                if browser.is_some() {
                    return Err(error("duplicate option"));
                }
                browser = Some(
                    Browser::parse(&take_value(args, &mut i, "--browser")?)
                        .ok_or_else(|| error("browser must be firefox or librewolf"))?,
                );
            }
            "--name" if action == "create" => {
                if name.is_some() {
                    return Err(error("duplicate option"));
                }
                name = Some(take_value(args, &mut i, "--name")?);
            }
            "--default" if action == "create" => {
                if default.is_some() {
                    return Err(error("duplicate option"));
                }
                default = Some(match take_value(args, &mut i, "--default")?.as_str() {
                    "yes" => true,
                    "no" => false,
                    _ => return Err(error("default must be yes or no")),
                });
            }
            "--template" if (action == "create" || action == "apply") && !template => {
                if take_value(args, &mut i, "--template")? != "strict" {
                    return Err(error("template must be strict"));
                }
                template = true;
            }
            "--backup" if action == "apply" && !backup => backup = true,
            "--yes" if action == "remove" && !yes => yes = true,
            "--private-window" if action == "launch" && !private => private = true,
            "--all" if action == "list" && !all => all = true,
            value if value.starts_with('-') => return Err(error("unknown or duplicate option")),
            value => positional.push(value),
        }
        i += 1;
    }
    let one_selector = || -> io::Result<Selector> {
        if positional.len() != 1 {
            return Err(error("expected one profile name or ID"));
        }
        selector(positional[0])
    };
    match action.as_str() {
        "list" if positional.is_empty() => Ok(Command::List { browser, all }),
        "create" => {
            if positional.len() > 1
                || positional
                    .first()
                    .is_some_and(|value| !value.starts_with('@'))
            {
                return Err(error("create accepts only an optional profile ID"));
            }
            Ok(Command::Create {
                id: positional
                    .first()
                    .map(|value| ProfileId::parse(value))
                    .transpose()?,
                browser,
                name,
                default,
            })
        }
        "apply" => Ok(Command::Apply {
            browser,
            selector: one_selector()?,
            backup,
        }),
        "default-get" if positional.is_empty() => Ok(Command::DefaultGet { browser }),
        "default-set" => Ok(Command::DefaultSet {
            browser,
            selector: one_selector()?,
        }),
        "remove" => Ok(Command::Remove {
            browser,
            selector: one_selector()?,
            yes,
        }),
        "launch" if positional.len() <= 1 => Ok(Command::Launch {
            browser,
            selector: positional
                .first()
                .map(|value| selector(value))
                .transpose()?,
            private,
            args: split.map(|at| args[at + 1..].to_vec()).unwrap_or_default(),
        }),
        _ => Err(error("invalid arguments")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_ids() {
        assert_eq!(ProfileId::parse("@0").unwrap(), ProfileId(0));
        assert_eq!(ProfileId::parse("@1").unwrap(), ProfileId(1));
        for id in ["@", "@01", "@-1", "#1", "1"] {
            assert!(ProfileId::parse(id).is_err());
        }
    }

    #[test]
    fn parses_selectors_and_create_options() {
        assert_eq!(
            parse(vec![
                "create".into(),
                "@0".into(),
                "--name".into(),
                "work".into(),
                "--browser".into(),
                "firefox".into(),
                "--default".into(),
                "no".into()
            ])
            .unwrap(),
            Command::Create {
                id: Some(ProfileId(0)),
                name: Some("work".into()),
                browser: Some(Browser::Firefox),
                default: Some(false)
            }
        );
        assert_eq!(
            parse(vec![
                "launch".into(),
                "@1".into(),
                "--".into(),
                "https://example.com".into()
            ])
            .unwrap(),
            Command::Launch {
                browser: None,
                selector: Some(Selector::Id(ProfileId(1))),
                private: false,
                args: vec!["https://example.com".into()]
            }
        );
    }

    #[test]
    fn bare_launch_and_numeric_names_are_not_ids() {
        assert_eq!(
            parse(vec!["launch".into(), "--private-window".into()]).unwrap(),
            Command::Launch {
                browser: None,
                selector: None,
                private: true,
                args: Vec::new(),
            }
        );
        assert_eq!(
            parse(vec!["launch".into(), "123".into()]).unwrap(),
            Command::Launch {
                browser: None,
                selector: Some(Selector::Name("123".into())),
                private: false,
                args: Vec::new(),
            }
        );
    }

    #[test]
    fn rejects_conflicts() {
        for args in [
            vec!["create", "work"],
            vec!["apply", "@01"],
            vec!["apply", "#1"],
            vec!["create", "@1", "@2"],
            vec![
                "default",
                "set",
                "work",
                "--browser",
                "firefox",
                "--browser",
                "librewolf",
            ],
        ] {
            assert!(parse(args.into_iter().map(String::from).collect()).is_err());
        }
    }
}
