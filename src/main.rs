use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

mod cli;

use cli::{Browser, Command};

const MARKER: &str = ".browserprofile-owned";

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
    match browser {
        Browser::Firefox => config_root().join("mozilla/firefox/profiles.ini"),
        Browser::Librewolf => env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(".librewolf/profiles.ini"))
            .unwrap_or_else(|| PathBuf::from(".librewolf/profiles.ini")),
    }
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
fn validate_user_js(dir: &Path, backup: bool) -> io::Result<()> {
    safe_dir(dir)?;
    let file = dir.join("user.js");
    if file.exists() {
        let file_type = fs::symlink_metadata(&file)?.file_type();
        if file_type.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "refusing symlinked user.js",
            ));
        }
        if !file_type.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "user.js must be a regular file",
            ));
        }
    }
    if backup && file.exists() {
        let backup_type = fs::symlink_metadata(dir.join("user.js.browserprofile-backup"))
            .map(|m| m.file_type())
            .ok();
        if backup_type.is_some_and(|ty| ty.is_symlink() || !ty.is_file()) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "backup must be a regular file",
            ));
        }
    }
    Ok(())
}
fn write_user_js(dir: &Path, browser: Browser, backup: bool) -> io::Result<()> {
    validate_user_js(dir, backup)?;
    let file = dir.join("user.js");
    if backup && file.exists() {
        let contents = fs::read(&file)?;
        atomic_write(&dir.join("user.js.browserprofile-backup"), &contents)?;
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
fn profiles_from_ini_at(ini: &Path) -> io::Result<Vec<(String, PathBuf)>> {
    reject_symlink_ancestors(ini)?;
    if !ini.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(ini)?;
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
        let path = resolve_ini_path(ini, raw_path, p.relative)?;
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
fn profiles_from_ini(browser: Browser) -> io::Result<Vec<(String, PathBuf)>> {
    profiles_from_ini_at(&ini_path(browser))
}
fn reconcile_librewolf() -> io::Result<()> {
    let obsolete = config_root().join("librewolf/librewolf/profiles.ini");
    if !obsolete.exists() {
        return Ok(());
    }
    let _lock = MutationLock::acquire(Browser::Librewolf)?;
    let candidates = profiles_from_ini_at(&obsolete)?
        .into_iter()
        .filter(|(name, path)| {
            valid_name(name)
                && path == &profile_dir(Browser::Librewolf, name)
                && safe_dir(path).is_ok()
                && owned_marker(path)
        })
        .collect::<Vec<_>>();
    let known = profiles_from_ini(Browser::Librewolf)?;
    for (name, path) in &candidates {
        for (known_name, known_path) in &known {
            if (name == known_name && !same_path(path, known_path))
                || (same_path(path, known_path) && name != known_name)
            {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "LibreWolf profile registry collision",
                ));
            }
        }
    }
    for (index, (name, path)) in candidates.iter().enumerate() {
        if candidates[..index]
            .iter()
            .any(|(known_name, known_path)| name == known_name || same_path(path, known_path))
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "LibreWolf profile registry collision",
            ));
        }
    }
    for (name, _) in candidates {
        register_profile_locked(Browser::Librewolf, &name)?;
    }
    Ok(())
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
fn is_owned_profile(browser: Browser, name: &str, path: &Path) -> bool {
    path == profile_dir(browser, name) && owned_marker(path)
}
fn protected_root(path: &Path) -> bool {
    path == Path::new("/")
        || env::var_os("HOME").is_some_and(|home| path == Path::new(&home))
        || path == data_root()
        || path == data_root().join("firefox")
        || path == data_root().join("librewolf")
        || path == config_root()
}
fn require_unambiguous_profile(
    profiles: &[(String, PathBuf)],
    name: &str,
    target: &Path,
) -> io::Result<()> {
    if profiles
        .iter()
        .any(|(known, path)| known != name && same_path(path, target))
    {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "profile path collision",
        ));
    }
    Ok(())
}
fn confirm_unmanaged(operation: &str, name: &str, dir: &Path) -> io::Result<bool> {
    print!(
        "{operation} unmanaged registered profile {name} at {}? [y|N] ",
        dir.display()
    );
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(answer.trim() == "y")
}
fn set_default(browser: Browser, name: &str) -> io::Result<()> {
    let _lock = MutationLock::acquire(browser)?;
    set_default_locked(browser, name, true)
}
fn set_default_locked(browser: Browser, name: &str, confirm: bool) -> io::Result<()> {
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
        discovered.iter().find(|(n, _)| n == name).cloned(),
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
    require_unambiguous_profile(&discovered, name, &target)?;
    safe_dir(&target)?;
    if protected_root(&target) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refusing protected profile root",
        ));
    }
    if !target.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "profile does not exist",
        ));
    }
    if confirm
        && !is_owned_profile(browser, name, &target)
        && !confirm_unmanaged("Set default for", name, &target)?
    {
        return Ok(());
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
fn register_profile_locked(browser: Browser, name: &str) -> io::Result<()> {
    let ini = ini_path(browser);
    let text = match fs::read_to_string(&ini) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let parsed = profiles(&text);
    if parsed.iter().any(|p| p.name.as_deref() == Some(name)) {
        return Ok(());
    }
    let target_path = profile_dir(browser, name).to_string_lossy().into_owned();
    let next = parsed
        .iter()
        .filter_map(|p| {
            p.section
                .strip_prefix("Profile")
                .and_then(|n| n.parse::<usize>().ok())
        })
        .max()
        .map_or(0, |n| n + 1);
    let mut out = text.lines().map(str::to_owned).collect::<Vec<_>>();
    out.extend([
        String::new(),
        format!("[Profile{next}]"),
        format!("Name={name}"),
        "IsRelative=0".into(),
        format!("Path={target_path}"),
        "Default=0".into(),
    ]);
    if let Some(parent) = ini.parent() {
        reject_symlink_ancestors(parent)?;
        fs::create_dir_all(parent)?;
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
            set_default_locked(browser, name, false)?;
        } else {
            register_profile_locked(browser, name)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&dir);
    }
    result
}
fn list_names(browser: Browser, all: bool) -> io::Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    let root = data_root().join(browser.as_str());
    if root.exists() {
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() && (all || owned_marker(&entry.path())) {
                names.insert(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    if all {
        names.extend(
            profiles_from_ini(browser)?
                .into_iter()
                .map(|(name, _)| name),
        );
    }
    Ok(names)
}
fn list(browser: Option<Browser>, all: bool) -> io::Result<()> {
    let browsers = match browser {
        Some(browser) => vec![browser],
        None => vec![Browser::Firefox, Browser::Librewolf],
    };
    let qualified = browser.is_none();
    for browser in browsers {
        if browser == Browser::Librewolf {
            reconcile_librewolf()?;
        }
        for name in list_names(browser, all)? {
            if qualified {
                println!("{}:{name}", browser.as_str());
            } else {
                println!("{name}");
            }
        }
    }
    Ok(())
}
fn unregister_profile_locked(browser: Browser, name: &str, target: &Path) -> io::Result<()> {
    let ini = ini_path(browser);
    reject_symlink_ancestors(&ini)?;
    let text = match fs::read_to_string(&ini) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let parsed = profiles(&text);
    let mut remove_sections = BTreeSet::new();
    for profile in &parsed {
        if profile.name.as_deref() == Some(name) {
            if let Some(path) = profile.path.as_deref() {
                if same_path(&resolve_ini_path(&ini, path, profile.relative)?, target) {
                    remove_sections.insert(profile.section.as_str());
                }
            }
        }
    }
    let mut out = Vec::new();
    let mut section = "";
    let mut changed = false;
    for line in text.lines() {
        if line.starts_with('[') && line.ends_with(']') {
            section = &line[1..line.len() - 1];
        }
        if remove_sections.contains(section) {
            changed = true;
            continue;
        }
        if is_install_section(section) {
            if let Some(path) = line.strip_prefix("Default=") {
                if same_path(
                    &resolve_ini_path(&ini, path, Path::new(path).is_relative())?,
                    target,
                ) {
                    changed = true;
                    continue;
                }
            }
        }
        out.push(line);
    }
    if changed {
        atomic_write(&ini, (out.join("\n") + "\n").as_bytes())?;
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
    let expected = profile_dir(browser, name);
    let registered_profiles = profiles_from_ini(browser)?;
    let registered = registered_profiles
        .iter()
        .find(|(known, _)| known == name)
        .cloned();
    let dir = match (is_owned_profile(browser, name, &expected), registered) {
        (true, Some((_, path))) if !same_path(&expected, &path) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "profile name collision",
            ));
        }
        (true, _) => expected,
        (false, Some((_, path))) => path,
        (false, None) => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "profile is not browserprofile-owned or registered",
            ));
        }
    };
    require_unambiguous_profile(&registered_profiles, name, &dir)?;
    safe_dir(&dir)?;
    if protected_root(&dir) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refusing protected profile root",
        ));
    }
    let owned = is_owned_profile(browser, name, &dir);
    if dir.exists() {
        if let Ok(metadata) = fs::symlink_metadata(dir.join(MARKER)) {
            if metadata.file_type().is_symlink() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "refusing symlinked ownership marker",
                ));
            }
            if !metadata.file_type().is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "ownership marker must be a regular file",
                ));
            }
        }
        if !dir.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "profile path is not a directory",
            ));
        }
        if owned_marker(&dir) && !owned {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "profile ownership mismatch",
            ));
        }
        validate_user_js(&dir, false)?;
    }
    let is_default = default_profile(browser)?
        .map(|(_, default_dir)| same_path(&default_dir, &dir))
        .unwrap_or(false);
    let confirmed = if !owned {
        confirm_unmanaged("Remove", name, &dir)?
    } else if is_default {
        print!("Remove browser default profile {name}? [y|N] ");
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        answer.trim() == "y"
    } else if !yes {
        print!("Remove {name}? [y/N] ");
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        answer.trim() == "y"
    } else {
        true
    };
    if !confirmed {
        return Ok(());
    }
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    unregister_profile_locked(browser, name, &dir)
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
    if protected_root(&dir) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refusing protected profile root",
        ));
    }
    validate_user_js(&dir, backup)?;
    let registered_profiles = profiles_from_ini(browser)?;
    let registered_name = registered_profiles
        .iter()
        .find(|(_, path)| same_path(path, &dir))
        .map(|(name, _)| name.clone());
    if let Some(registered_name) = registered_name {
        require_unambiguous_profile(&registered_profiles, &registered_name, &dir)?;
        if !is_owned_profile(browser, &registered_name, &dir)
            && !confirm_unmanaged("Apply to", &registered_name, &dir)?
        {
            return Ok(());
        }
    }
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
    ProcessCommand::new(browser.as_str())
        .args(&argv)
        .spawn()
        .map(|_| ())
}
fn run() -> io::Result<()> {
    let command = cli::parse(env::args().skip(1).collect())?;
    match command {
        Command::List { browser, all } => list(browser, all),
        command => {
            if command.browser() == Some(Browser::Librewolf) {
                reconcile_librewolf()?;
            }
            match command {
                Command::Create {
                    browser,
                    name,
                    default,
                } => create(browser, &name, default),
                Command::Apply {
                    browser,
                    name,
                    backup,
                } => apply(browser, &name, backup),
                Command::DefaultGet { browser } => match default_profile(browser)? {
                    Some((name, _)) => {
                        println!("{name}");
                        Ok(())
                    }
                    None => Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        "no default profile",
                    )),
                },
                Command::DefaultSet { browser, name } => set_default(browser, &name),
                Command::Remove { browser, name, yes } => remove(browser, &name, yes),
                Command::Launch {
                    browser,
                    name,
                    private,
                    args,
                } => launch(browser, &name, private, &args),
                Command::List { .. } => unreachable!(),
            }
        }
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("bp: {error}");
        if error.kind() == io::ErrorKind::InvalidInput {
            cli::usage();
            std::process::exit(2);
        }
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_reject_duplicate_fields() {
        let profile = &profiles("[Profile0]\nName=one\nName=two\nIsRelative=0\nPath=/tmp/one\n")[0];
        assert!(!profile.fields_valid);
    }

    #[test]
    fn ini_paths_reject_traversal() {
        assert!(resolve_ini_path(Path::new("/tmp/profiles.ini"), "../outside", true).is_err());
    }
}
