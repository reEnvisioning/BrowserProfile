use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

const MARKER: &str = ".browserprofile-owned";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Browser {
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
    fn as_str(self) -> &'static str {
        match self {
            Self::Firefox => "firefox",
            Self::Librewolf => "librewolf",
        }
    }
    fn config_dir(self) -> &'static str {
        match self {
            Self::Firefox => "mozilla/firefox",
            Self::Librewolf => "librewolf",
        }
    }
    fn executable(self) -> &'static str {
        self.as_str()
    }
}

fn xdg(name: &str, fallback: &str) -> PathBuf {
    env::var_os(name).map(PathBuf::from).unwrap_or_else(|| {
        env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(fallback))
            .unwrap_or_else(|| PathBuf::from(fallback))
    })
}
fn data_root() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join("browserprofile")
}
fn config_root() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config")
}
fn profile_dir(browser: Browser, name: &str) -> PathBuf {
    data_root().join(browser.as_str()).join(name)
}
fn ini_path(browser: Browser) -> PathBuf {
    let xdg_path = config_root()
        .join(browser.config_dir())
        .join("profiles.ini");
    if browser == Browser::Librewolf && !xdg_path.exists() {
        if let Some(home) = env::var_os("HOME") {
            let legacy = PathBuf::from(home).join(".librewolf/profiles.ini");
            if legacy.exists() {
                return legacy;
            }
        }
    }
    xdg_path
}
fn reject_symlink_ancestors(path: &Path) -> io::Result<()> {
    let mut current = Some(path);
    while let Some(p) = current {
        if fs::symlink_metadata(p)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "refusing symlinked path",
            ));
        }
        current = p.parent();
    }
    Ok(())
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
}
fn owned_marker(dir: &Path) -> bool {
    fs::symlink_metadata(dir.join(MARKER))
        .map(|m| m.file_type().is_file())
        .unwrap_or(false)
}
fn safe_dir(path: &Path) -> io::Result<()> {
    if path
        .components()
        .any(|c| c == std::path::Component::ParentDir)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "refusing path traversal",
        ));
    }
    reject_symlink_ancestors(path)?;
    if path.exists() && !path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "profile path is not a directory",
        ));
    }
    Ok(())
}
fn template(browser: Browser) -> &'static str {
    match browser {
        Browser::Firefox => include_str!("../templates/firefox.user.js"),
        Browser::Librewolf => include_str!("../templates/librewolf.user.js"),
    }
}
fn write_user_js(dir: &Path, browser: Browser, backup: bool) -> io::Result<()> {
    safe_dir(dir)?;
    let file = dir.join("user.js");
    if file.is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refusing symlinked user.js",
        ));
    }
    let backup_file = dir.join("user.js.browserprofile-backup");
    if backup && file.exists() {
        let file_type = fs::symlink_metadata(&file)?.file_type();
        if !file_type.is_file() || file_type.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "user.js must be a regular file",
            ));
        }
        let backup_type = fs::symlink_metadata(&backup_file)
            .map(|m| m.file_type())
            .ok();
        if backup_type.is_some_and(|ty| ty.is_symlink() || !ty.is_file()) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "backup must be a regular file",
            ));
        }
        let contents = fs::read(&file)?;
        atomic_write(&backup_file, &contents)?;
    }
    atomic_write(&file, template(browser).as_bytes())
}
fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
    reject_symlink_ancestors(parent)?;
    if path.is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refusing symlinked file",
        ));
    }
    for n in 0..100 {
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no filename"))?;
        let tmp = parent.join(format!(".{}.tmp-{n}", name.to_string_lossy()));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(mut f) => {
                let result = (|| {
                    f.write_all(contents)?;
                    f.sync_all()?;
                    drop(f);
                    fs::rename(&tmp, path)
                })();
                if result.is_err() {
                    let _ = fs::remove_file(&tmp);
                }
                return result;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create temporary file",
    ))
}

struct MutationLock {
    path: PathBuf,
}
impl MutationLock {
    fn acquire(browser: Browser) -> io::Result<Self> {
        let root = data_root().join(browser.as_str());
        fs::create_dir_all(&root)?;
        let path = root.join(".browserprofile.lock");
        fs::create_dir(&path).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "another browserprofile mutation is running",
                )
            } else {
                error
            }
        })?;
        Ok(Self { path })
    }
}
impl Drop for MutationLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}

#[derive(Default, Debug)]
struct IniProfile {
    section: String,
    name: Option<String>,
    path: Option<String>,
    relative: bool,
    relative_present: bool,
    relative_valid: bool,
    name_present: bool,
    path_present: bool,
    default_present: bool,
    default: bool,
    fields_valid: bool,
}
fn is_profile_section(section: &str) -> bool {
    section
        .strip_prefix("Profile")
        .is_some_and(|suffix| !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()))
}
fn is_install_section(section: &str) -> bool {
    section.starts_with("Install") && section.len() > "Install".len()
}
fn profiles(ini: &str) -> Vec<IniProfile> {
    let mut out = Vec::new();
    let mut cur: Option<IniProfile> = None;
    for line in ini.lines() {
        if line.starts_with('[') && line.ends_with(']') {
            if let Some(p) = cur.take() {
                if is_profile_section(&p.section) {
                    out.push(p);
                }
            }
            cur = Some(IniProfile {
                section: line[1..line.len() - 1].to_string(),
                relative_valid: false,
                fields_valid: true,
                ..Default::default()
            });
        } else if let Some((k, v)) = line.split_once('=') {
            if let Some(p) = cur.as_mut() {
                match k {
                    "Name" => {
                        p.fields_valid &= !p.name_present;
                        p.name_present = true;
                        p.name = Some(v.to_string());
                    }
                    "Path" => {
                        p.fields_valid &= !p.path_present;
                        p.path_present = true;
                        p.path = Some(v.to_string());
                    }
                    "IsRelative" => {
                        let duplicate = p.relative_present;
                        p.relative_present = true;
                        match v {
                            "0" => {
                                p.relative = false;
                                p.relative_valid = true;
                            }
                            "1" => {
                                p.relative = true;
                                p.relative_valid = true;
                            }
                            _ => p.relative_valid = false,
                        }
                        if duplicate {
                            p.relative_valid = false;
                        }
                    }
                    "Default" => {
                        p.fields_valid &= !p.default_present && matches!(v, "0" | "1");
                        p.default_present = true;
                        p.default = v == "1";
                    }
                    _ => {}
                }
            }
        }
    }
    if let Some(p) = cur {
        if is_profile_section(&p.section) {
            out.push(p);
        }
    }
    out
}
fn profiles_from_ini(browser: Browser) -> io::Result<Vec<(String, PathBuf)>> {
    let ini = ini_path(browser);
    reject_symlink_ancestors(&ini)?;
    if !ini.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&ini)?;
    let mut out = Vec::new();
    for p in profiles(&text) {
        if !p.fields_valid || !p.relative_present || !p.relative_valid {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "duplicate or invalid profile field",
            ));
        }
        let Some(raw_path) = p.path.as_deref() else {
            continue;
        };
        let path = resolve_ini_path(&ini, raw_path, p.relative)?;
        let Some(name) = p
            .name
            .or_else(|| path.file_name().map(|n| n.to_string_lossy().into_owned()))
        else {
            continue;
        };
        if out.iter().any(|(known, _)| known == &name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "duplicate profile name",
            ));
        }
        out.push((name, path));
    }
    Ok(out)
}
fn default_profile(browser: Browser) -> io::Result<Option<(String, PathBuf)>> {
    let path = ini_path(browser);
    reject_symlink_ancestors(&path)?;
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path)?;
    let ps = profiles(&text);
    let ini = ini_path(browser);
    let install_paths = install_default_paths(&ini, &text)?;
    // Firefox can leave several Install sections behind. Require one
    // authoritative path that names a profile, then use Profile defaults as
    // a deterministic fallback if none do.
    let mut resolved_profiles = Vec::new();
    for p in &ps {
        if !p.fields_valid || !p.relative_present || !p.relative_valid {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "duplicate or invalid profile field",
            ));
        }
        if let Some(raw) = p.path.as_deref() {
            resolved_profiles.push((p, resolve_ini_path(&ini, raw, p.relative)?));
        }
    }
    let install_profiles: Vec<&IniProfile> = install_paths
        .iter()
        .filter_map(|target| {
            resolved_profiles
                .iter()
                .find(|(_, path)| same_path(path, target))
                .map(|(profile, _)| *profile)
        })
        .collect();
    if install_profiles.len() > 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "conflicting install defaults",
        ));
    }
    let install_profile = install_profiles.into_iter().next();
    let profile_defaults: Vec<&IniProfile> = ps.iter().filter(|p| p.default).collect();
    if install_profile.is_none() && profile_defaults.len() > 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "conflicting profile defaults",
        ));
    }
    let p = install_profile.or_else(|| profile_defaults.into_iter().next());
    if let Some(p) = p {
        let path = resolve_ini_path(
            &path,
            p.path
                .as_deref()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "profile has no path"))?,
            p.relative,
        )?;
        safe_dir(&path)?;
        let name = p
            .name
            .clone()
            .or_else(|| path.file_name().map(|n| n.to_string_lossy().into_owned()))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "profile has no name"))?;
        return Ok(Some((name, path)));
    }
    Ok(None)
}
fn install_default_paths(ini: &Path, text: &str) -> io::Result<Vec<PathBuf>> {
    let mut in_install = false;
    let mut paths = Vec::new();
    for line in text.lines() {
        if line.starts_with('[') && line.ends_with(']') {
            in_install = is_install_section(&line[1..line.len() - 1]);
        } else if in_install {
            if let Some(path) = line.strip_prefix("Default=") {
                paths.push(resolve_ini_path(ini, path, Path::new(path).is_relative())?);
            }
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}
fn resolve_ini_path(ini: &Path, path: &str, relative: bool) -> io::Result<PathBuf> {
    let raw = Path::new(path);
    if path.is_empty() || raw == Path::new(".") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "profile path is empty or current directory",
        ));
    }
    if raw.is_absolute() == relative {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "profile path does not match IsRelative",
        ));
    }
    if raw
        .components()
        .any(|component| component == std::path::Component::ParentDir)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "profile path contains traversal",
        ));
    }
    Ok(if relative {
        ini.parent().unwrap_or_else(|| Path::new(".")).join(raw)
    } else {
        raw.to_path_buf()
    })
}
fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}
fn resolve(browser: Browser, name: &str) -> io::Result<PathBuf> {
    if name == "default" {
        return default_profile(browser)?
            .map(|(_, p)| p)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no browser default profile"));
    }
    if !valid_name(name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid profile name",
        ));
    }
    let owned = profile_dir(browser, name);
    let discovered = profiles_from_ini(browser)?
        .into_iter()
        .find(|p| p.0 == name);
    if owned.is_dir() && owned_marker(&owned) {
        if let Some((_, path)) = &discovered {
            if !same_path(&owned, path) {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "profile name collision",
                ));
            }
        }
        return Ok(owned);
    }
    if let Some((_, path)) = discovered {
        return Ok(path);
    }
    if owned.exists() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unmanaged profile is not registered in profiles.ini",
        ));
    }
    Ok(owned)
}
fn set_default(browser: Browser, name: &str) -> io::Result<()> {
    let _lock = MutationLock::acquire(browser)?;
    set_default_locked(browser, name)
}
fn set_default_locked(browser: Browser, name: &str) -> io::Result<()> {
    if !valid_name(name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid profile name",
        ));
    }
    let ini = ini_path(browser);
    let discovered = profiles_from_ini(browser)?;
    let owned = profile_dir(browser, name);
    let target = match (
        owned.is_dir(),
        discovered.into_iter().find(|(n, _)| n == name),
    ) {
        (true, Some((_, path))) if !same_path(&owned, &path) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "profile name collision",
            ));
        }
        (true, _) => owned,
        (false, Some((_, path))) => path,
        (false, None) => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "profile does not exist",
            ))
        }
    };
    safe_dir(&target)?;
    if !target.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "profile does not exist",
        ));
    }
    reject_symlink_ancestors(&ini)?;
    let text = if ini.exists() {
        fs::read_to_string(&ini)?
    } else {
        String::new()
    };
    let parsed = profiles(&text);
    let target_section = parsed
        .iter()
        .filter(|p| {
            p.path
                .as_deref()
                .map(|profile_path| {
                    resolve_ini_path(&ini, profile_path, p.relative)
                        .map(|path| same_path(&path, &target))
                        .unwrap_or(false)
                })
                .unwrap_or(false)
        })
        .min_by_key(|p| (p.name.as_deref() != Some(name), p.section.as_str()))
        .map(|p| p.section.clone());
    let target_path = target.to_string_lossy().into_owned();
    let mut out = Vec::new();
    let mut section = String::new();
    let mut body: Vec<String> = Vec::new();
    let flush = |section: &str, body: &mut Vec<String>, out: &mut Vec<String>| {
        if section.is_empty() {
            out.append(body);
            return;
        }
        let is_profile = section
            .strip_prefix('[')
            .and_then(|name| name.strip_suffix(']'))
            .is_some_and(is_profile_section);
        let target_section = target_section
            .as_deref()
            .is_some_and(|s| format!("[{s}]") == section);
        let mut default = false;
        for line in body.iter_mut() {
            if is_profile && line.starts_with("Default=") {
                *line = format!("Default={}", if target_section { "1" } else { "0" });
                default = true;
            }
            if section
                .strip_prefix('[')
                .and_then(|name| name.strip_suffix(']'))
                .is_some_and(is_install_section)
                && line.starts_with("Default=")
            {
                *line = format!("Default={target_path}");
                default = true;
            }
        }
        if is_profile && target_section && !default {
            body.push("Default=1".into());
        }
        if section
            .strip_prefix('[')
            .and_then(|name| name.strip_suffix(']'))
            .is_some_and(is_install_section)
            && !default
        {
            body.push(format!("Default={target_path}"));
        }
        out.push(section.to_owned());
        out.append(body);
    };
    for line in text.lines() {
        if line.starts_with('[') && line.ends_with(']') {
            flush(&section, &mut body, &mut out);
            section = line.to_owned();
        } else {
            body.push(line.to_owned());
        }
    }
    flush(&section, &mut body, &mut out);
    if target_section.is_none() {
        let next = parsed
            .iter()
            .filter_map(|p| {
                p.section
                    .strip_prefix("Profile")
                    .and_then(|n| n.parse::<usize>().ok())
            })
            .max()
            .map_or(0, |n| n + 1);
        out.extend([
            String::new(),
            format!("[Profile{next}]"),
            format!("Name={name}"),
            "IsRelative=0".into(),
            format!("Path={target_path}"),
            "Default=1".into(),
        ]);
    }
    if !ini.exists() {
        if let Some(parent) = ini.parent() {
            reject_symlink_ancestors(parent)?;
            fs::create_dir_all(parent)?;
        }
    }
    atomic_write(&ini, (out.join("\n") + "\n").as_bytes())
}
fn create(browser: Browser, name: &str, make_default: bool) -> io::Result<()> {
    let _lock = MutationLock::acquire(browser)?;
    create_locked(browser, name, make_default)
}
fn create_locked(browser: Browser, name: &str, make_default: bool) -> io::Result<()> {
    if !valid_name(name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid profile name",
        ));
    }
    let dir = profile_dir(browser, name);
    safe_dir(&dir)?;
    let discovered = profiles_from_ini(browser)?;
    if dir.exists()
        || discovered.iter().any(|(known, _)| known == name)
        || discovered.iter().any(|(_, path)| same_path(path, &dir))
    {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "profile already exists",
        ));
    }
    fs::create_dir_all(&dir)?;
    let result = (|| {
        write_user_js(&dir, browser, false)?;
        // The marker is the last profile-local step, before the registry can
        // change. Any later failure removes the whole new directory.
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(MARKER))?;
        if make_default {
            set_default_locked(browser, name)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&dir);
    }
    result
}
fn list(browser: Browser, all: bool) -> io::Result<()> {
    let root = data_root().join(browser.as_str());
    if root.exists() {
        for e in fs::read_dir(root)? {
            let e = e?;
            if e.file_type()?.is_dir()
                && (all
                    || fs::symlink_metadata(e.path().join(MARKER))
                        .map(|m| m.is_file() && !m.file_type().is_symlink())
                        .unwrap_or(false))
            {
                println!("{}", e.file_name().to_string_lossy());
            }
        }
    }
    if all {
        for (name, _) in profiles_from_ini(browser)? {
            println!("{name}");
        }
    }
    Ok(())
}
fn remove(browser: Browser, name: &str, yes: bool) -> io::Result<()> {
    let _lock = MutationLock::acquire(browser)?;
    remove_locked(browser, name, yes)
}
fn remove_locked(browser: Browser, name: &str, yes: bool) -> io::Result<()> {
    if !valid_name(name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid profile name",
        ));
    }
    let dir = profile_dir(browser, name);
    safe_dir(&dir)?;
    if !dir.join(MARKER).is_file() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "profile is not browserprofile-owned",
        ));
    }
    let ini = ini_path(browser);
    if ini.exists() {
        let text = fs::read_to_string(&ini)?;
        if install_default_paths(&ini, &text)?
            .iter()
            .any(|install_dir| same_path(install_dir, &dir))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "refusing to remove an install default profile",
            ));
        }
    }
    if let Some((_, default_dir)) = default_profile(browser)? {
        if same_path(&default_dir, &dir) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "refusing to remove the browser default profile",
            ));
        }
    }
    if dir.join("user.js").is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refusing symlinked user.js",
        ));
    }
    if dir.join(MARKER).is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refusing symlinked ownership marker",
        ));
    }
    if !yes {
        print!("Remove {name}? [y/N] ");
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if answer.trim() != "y" {
            return Ok(());
        }
    }
    fs::remove_dir_all(dir)
}
fn browser_argv(dir: &Path, private: bool, args: &[String]) -> Vec<String> {
    let mut argv = vec![
        "--profile".into(),
        dir.to_string_lossy().into_owned(),
        "--no-remote".into(),
    ];
    if private {
        argv.push("--private-window".into());
    }
    argv.extend_from_slice(args);
    argv
}
fn apply(browser: Browser, name: &str, backup: bool) -> io::Result<()> {
    let _lock = MutationLock::acquire(browser)?;
    let dir = resolve(browser, name)?;
    write_user_js(&dir, browser, backup)
}
fn launch(browser: Browser, target: &str, private: bool, args: &[String]) -> io::Result<()> {
    let dir = resolve(browser, target)?;
    safe_dir(&dir)?;
    if !dir.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "profile does not exist",
        ));
    }
    let argv = browser_argv(&dir, private, args);
    Command::new(browser.executable())
        .args(&argv)
        .spawn()
        .map(|_| ())
}
fn usage() {
    eprintln!("usage: bp <list|create|apply|default|get|set|remove|launch> ...");
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
fn validate_cli(command: &str, args: &[String]) -> io::Result<()> {
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
fn main() -> io::Result<()> {
    let mut a: Vec<String> = env::args().skip(1).collect();
    if a.is_empty() {
        usage();
        return Ok(());
    }
    let command = a.remove(0);
    validate_cli(&command, &a)?;
    let before_separator = a.iter().position(|x| x == "--").unwrap_or(a.len());
    let browser = option(&a[..before_separator], "--browser", Browser::Firefox)?;
    let positional = positional_args(&a);
    match command.as_str() {
        "list" => list(browser, a.iter().any(|x| x == "--all")),
        "create" => {
            let n = positional
                .first()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing name"))?;
            create(browser, n, a.iter().any(|x| x == "--default"))
        }
        "apply" => {
            let n = positional
                .first()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing name"))?;
            apply(browser, n, a.iter().any(|x| x == "--backup"))
        }
        "remove" => {
            let n = positional
                .first()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing name"))?;
            remove(browser, n, a.iter().any(|x| x == "--yes"))
        }
        "launch" => {
            let n = positional
                .first()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing name"))?;
            let split = a.iter().position(|x| x == "--");
            let extra = split.map(|i| &a[i + 1..]).unwrap_or(&[]);
            launch(
                browser,
                n,
                a[..before_separator]
                    .iter()
                    .any(|x| x == "--private-window"),
                extra,
            )
        }
        "default" => {
            let sub = positional.first().copied().unwrap_or("");
            match sub {
                "get" => match default_profile(browser)? {
                    Some((n, _)) => {
                        println!("{n}");
                        Ok(())
                    }
                    None => Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        "no default profile",
                    )),
                },
                "set" => {
                    let n = positional.get(1).ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidInput, "missing name")
                    })?;
                    set_default(browser, n)
                }
                _ => Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "use default get or default set",
                )),
            }
        }
        _ => {
            usage();
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unknown command",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }
    fn temp() -> PathBuf {
        let p = env::temp_dir().join(format!(
            "bp-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&p).unwrap();
        p
    }
    #[test]
    fn names_are_safe() {
        assert!(valid_name("work_1"));
        assert!(!valid_name("../x"));
        assert!(!valid_name(".hidden"));
        assert!(!valid_name("a/b"));
    }
    #[test]
    fn malformed_ini_paths_are_rejected_before_resolution() {
        let ini = Path::new("/tmp/profiles.ini");
        for (path, relative) in [
            ("", true),
            (".", true),
            ("/absolute", true),
            ("relative", false),
            ("../escape", true),
        ] {
            assert!(
                resolve_ini_path(ini, path, relative).is_err(),
                "{path:?}, {relative}"
            );
        }
        assert!(resolve_ini_path(ini, "profile", true).is_ok());
        assert!(resolve_ini_path(ini, "/profile", false).is_ok());
        assert!(!profiles("[Profile0]\nIsRelative=2\nPath=/profile\n")[0].relative_valid);
    }
    #[test]
    fn profile_is_relative_must_be_present_once() {
        let missing = profiles("[Profile0]\nPath=profile\n");
        assert!(!missing[0].relative_present);
        assert!(!missing[0].relative_valid);
        let duplicate = profiles("[Profile0]\nIsRelative=1\nIsRelative=1\nPath=profile\n");
        assert!(duplicate[0].relative_present);
        assert!(!duplicate[0].relative_valid);
        assert!(profiles("[Profile0]\nIsRelative=1\nPath=profile\n")[0].relative_valid);
        assert!(profiles("[Profile0]\nIsRelative=0\nPath=/profile\n")[0].relative_valid);
        assert!(
            !profiles("[Profile0]\nName=a\nName=b\nIsRelative=0\nPath=/profile\n")[0].fields_valid
        );
        assert!(!profiles("[Profile0]\nIsRelative=0\nPath=/profile\nDefault=2\n")[0].fields_valid);
    }

    #[test]
    fn cli_rejects_unknown_and_bad_template_options() {
        assert!(validate_cli("create", &["name".into(), "--wat".into()]).is_err());
        assert!(validate_cli("list", &["--all".into()]).is_ok());
        assert!(validate_cli("create", &["name".into(), "--default".into()]).is_ok());
        assert!(validate_cli("apply", &["name".into(), "--backup".into()]).is_ok());
        assert!(validate_cli("remove", &["name".into(), "--yes".into()]).is_ok());
        assert!(validate_cli("launch", &["name".into(), "--private-window".into()]).is_ok());
        assert!(validate_cli(
            "apply",
            &["name".into(), "--template".into(), "loose".into()]
        )
        .is_err());
        assert!(validate_cli("list", &["--browser".into()]).is_err());
        assert!(validate_cli("list", &["--all".into(), "--all".into()]).is_err());
        assert!(validate_cli("default", &["get".into(), "extra".into()]).is_err());
    }
    #[test]
    fn argv_has_no_shell() {
        let p = PathBuf::from("a profile");
        let argv = browser_argv(&p, true, &["https://example.org".into()]);
        assert_eq!(
            argv,
            [
                "--profile",
                "a profile",
                "--no-remote",
                "--private-window",
                "https://example.org"
            ]
        );
    }
    #[test]
    fn owned_lifecycle_and_backup() {
        let _lock = env_lock();
        let root = temp();
        env::set_var("XDG_DATA_HOME", &root);
        let d = profile_dir(Browser::Firefox, "one");
        create(Browser::Firefox, "one", false).unwrap();
        fs::write(d.join("user.js"), "old").unwrap();
        write_user_js(&d, Browser::Firefox, true).unwrap();
        assert_eq!(
            fs::read_to_string(d.join("user.js.browserprofile-backup")).unwrap(),
            "old"
        );
        remove(Browser::Firefox, "one", true).unwrap();
        assert!(!d.exists());
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn default_ini_round_trip() {
        let _lock = env_lock();
        let root = temp();
        env::set_var("XDG_CONFIG_HOME", &root);
        env::set_var("XDG_DATA_HOME", &root);
        fs::create_dir_all(root.join("browserprofile/firefox/one")).unwrap();
        set_default(Browser::Firefox, "one").unwrap();
        assert_eq!(default_profile(Browser::Firefox).unwrap().unwrap().0, "one");
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn legacy_librewolf_relative_paths_use_ini_parent() {
        let _lock = env_lock();
        let root = temp();
        let home = root.join("home");
        fs::create_dir_all(home.join(".librewolf/profile")).unwrap();
        env::set_var("HOME", &home);
        env::set_var("XDG_CONFIG_HOME", root.join("config"));
        fs::write(
            home.join(".librewolf/profiles.ini"),
            "[Profile0]\nName=old\nIsRelative=1\nPath=profile\nDefault=1\n",
        )
        .unwrap();
        assert_eq!(
            default_profile(Browser::Librewolf).unwrap().unwrap().1,
            home.join(".librewolf/profile")
        );
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn discovered_profile_can_become_default() {
        let _lock = env_lock();
        let root = temp();
        let config = root.join("config");
        let profile = root.join("discovered");
        fs::create_dir_all(&profile).unwrap();
        fs::create_dir_all(config.join("mozilla/firefox")).unwrap();
        env::set_var("XDG_CONFIG_HOME", &config);
        env::set_var("XDG_DATA_HOME", root.join("data"));
        fs::write(
            config.join("mozilla/firefox/profiles.ini"),
            format!(
                "[Profile7]\nName=found\nIsRelative=0\nPath={}\n",
                profile.display()
            ),
        )
        .unwrap();
        set_default(Browser::Firefox, "found").unwrap();
        assert_eq!(
            default_profile(Browser::Firefox).unwrap().unwrap().0,
            "found"
        );
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn install_default_is_authoritative_and_create_rejects_name_collision() {
        let _lock = env_lock();
        let root = temp();
        let config = root.join("config");
        fs::create_dir_all(&config.join("mozilla/firefox")).unwrap();
        env::set_var("XDG_CONFIG_HOME", &config);
        env::set_var("XDG_DATA_HOME", root.join("data"));
        let ini = config.join("mozilla/firefox/profiles.ini");
        fs::write(
            &ini,
            "[Installabc]\nDefault=missing\n[Profile0]\nName=other\nPath=other\nIsRelative=1\nDefault=1\n",
        )
        .unwrap();
        assert_eq!(
            default_profile(Browser::Firefox).unwrap().unwrap().0,
            "other"
        );
        assert!(create(Browser::Firefox, "other", false).is_err());
        assert!(!profile_dir(Browser::Firefox, "other").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn distinct_install_defaults_are_rejected_deterministically() {
        let _lock = env_lock();
        let root = temp();
        let config = root.join("config");
        let first = root.join("a");
        let second = root.join("b");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::create_dir_all(config.join("mozilla/firefox")).unwrap();
        env::set_var("XDG_CONFIG_HOME", &config);
        env::set_var("XDG_DATA_HOME", root.join("data"));
        fs::write(
            config.join("mozilla/firefox/profiles.ini"),
            format!(
                "[Installsecond]\nDefault={}\n[Installfirst]\nDefault={}\n[Profile0]\nName=first\nIsRelative=0\nPath={}\n[Profile1]\nName=second\nIsRelative=0\nPath={}\n",
                second.display(),
                first.display(),
                first.display(),
                second.display()
            ),
        )
        .unwrap();
        assert!(default_profile(Browser::Firefox).is_err());
        let paths = install_default_paths(
            &config.join("mozilla/firefox/profiles.ini"),
            &fs::read_to_string(config.join("mozilla/firefox/profiles.ini")).unwrap(),
        )
        .unwrap();
        assert_eq!(paths, vec![first, second]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn every_install_default_protects_removal() {
        let _lock = env_lock();
        let root = temp();
        let config = root.join("config");
        env::set_var("XDG_CONFIG_HOME", &config);
        env::set_var("XDG_DATA_HOME", root.join("data"));
        create(Browser::Firefox, "protected-one", false).unwrap();
        create(Browser::Firefox, "protected-two", false).unwrap();
        let first = profile_dir(Browser::Firefox, "protected-one");
        let second = profile_dir(Browser::Firefox, "protected-two");
        fs::create_dir_all(config.join("mozilla/firefox")).unwrap();
        fs::write(
            config.join("mozilla/firefox/profiles.ini"),
            format!(
                "[Installone]\nDefault={}\n[Installtwo]\nDefault={}\n[Profile0]\nName=one\nIsRelative=0\nPath={}\n[Profile1]\nName=two\nIsRelative=0\nPath={}\n",
                first.display(),
                second.display(),
                first.display(),
                second.display()
            ),
        )
        .unwrap();
        assert!(default_profile(Browser::Firefox).is_err());
        assert!(remove(Browser::Firefox, "protected-one", true).is_err());
        assert!(remove(Browser::Firefox, "protected-two", true).is_err());
        assert!(first.exists());
        assert!(second.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn registry_symlink_ancestor_is_rejected_and_create_rolls_back() {
        use std::os::unix::fs::symlink;
        let _lock = env_lock();
        let root = temp();
        let real = root.join("real");
        let link = root.join("link");
        fs::create_dir_all(&real).unwrap();
        symlink(&real, &link).unwrap();
        env::set_var("XDG_DATA_HOME", root.join("data"));
        env::set_var("XDG_CONFIG_HOME", &link);
        assert!(create(Browser::Firefox, "rollback", true).is_err());
        assert!(!profile_dir(Browser::Firefox, "rollback").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn mutation_lock_prevents_create_from_touching_existing_profile() {
        let _lock = env_lock();
        let root = temp();
        env::set_var("XDG_DATA_HOME", root.join("data"));
        let _held = MutationLock::acquire(Browser::Firefox).unwrap();
        let dir = profile_dir(Browser::Firefox, "race");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(MARKER), "owned").unwrap();
        assert!(create(Browser::Firefox, "race", false).is_err());
        assert_eq!(fs::read_to_string(dir.join(MARKER)).unwrap(), "owned");
        let _ = fs::remove_dir_all(root);
    }
}
