use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

mod cli;

use cli::{Browser, Command, ProfileId, Selector};

const MARKER: &str = ".browserprofile-owned";
const MAX_TEMPLATE_BYTES: u64 = 1024 * 1024;

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
        Browser::Firefox => env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(".mozilla/firefox/profiles.ini"))
            .unwrap_or_else(|| PathBuf::from(".mozilla/firefox/profiles.ini")),
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
        && name != "default"
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
fn read_template(path: Option<&str>) -> io::Result<Vec<u8>> {
    let Some(path) = path else {
        return Ok(include_bytes!("../templates/template.user.js").to_vec());
    };
    let path = Path::new(path);
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "template must be a regular file",
        ));
    }
    let mut file = fs::File::open(path)?;
    let opened = file.metadata()?;
    let current = fs::symlink_metadata(path)?;
    if !opened.is_file()
        || !current.file_type().is_file()
        || opened.dev() != current.dev()
        || opened.ino() != current.ino()
        || opened.len() > MAX_TEMPLATE_BYTES
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "template must be an unchanged regular file no larger than 1 MiB",
        ));
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    Read::by_ref(&mut file)
        .take(MAX_TEMPLATE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_TEMPLATE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "template must be no larger than 1 MiB",
        ));
    }
    Ok(bytes)
}
fn template_name(path: Option<&str>) -> io::Result<String> {
    let path = Path::new(path.unwrap_or("template.user.js"));
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid template filename"))?;
    let name = name
        .strip_suffix(".user.js")
        .map(str::to_owned)
        .or_else(|| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| name.to_owned());
    if !valid_name(&name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid profile name",
        ));
    }
    Ok(name)
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
fn write_user_js(dir: &Path, template: &[u8], backup: bool) -> io::Result<()> {
    validate_user_js(dir, backup)?;
    let file = dir.join("user.js");
    if backup && file.exists() {
        let contents = fs::read(&file)?;
        atomic_write(&dir.join("user.js.browserprofile-backup"), &contents)?;
    }
    atomic_write(&file, template)
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
        reject_symlink_ancestors(&root)?;
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

struct CatalogLock {
    _lock: MutationLock,
}
impl CatalogLock {
    fn acquire() -> io::Result<Self> {
        let root = data_root();
        reject_symlink_ancestors(&root)?;
        fs::create_dir_all(&root)?;
        let path = root.join(".browserprofile.catalog.lock");
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
        Ok(Self {
            _lock: MutationLock { path },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CatalogEntry {
    id: ProfileId,
    browser: Browser,
    name: String,
}
fn catalog_path() -> PathBuf {
    data_root().join("profiles.catalog")
}
fn read_catalog() -> io::Result<Vec<CatalogEntry>> {
    let path = catalog_path();
    reject_symlink_ancestors(&path)?;
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let Some(text) = text.strip_suffix('\n') else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid profile catalog",
        ));
    };
    let mut entries = Vec::new();
    for line in text.split('\n') {
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid profile catalog",
            ));
        }
        let entry = CatalogEntry {
            id: ProfileId::parse(fields[0]).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid profile catalog")
            })?,
            browser: Browser::parse(fields[1]).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid profile catalog")
            })?,
            name: fields[2].into(),
        };
        if !valid_name(&entry.name)
            || entries.iter().any(|known: &CatalogEntry| {
                known.id == entry.id || (known.browser == entry.browser && known.name == entry.name)
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "duplicate or invalid profile catalog entry",
            ));
        }
        entries.push(entry);
    }
    Ok(entries)
}
fn write_catalog(entries: &[CatalogEntry]) -> io::Result<()> {
    let mut entries = entries.to_vec();
    entries.sort_by_key(|entry| entry.id);
    let text = entries
        .into_iter()
        .map(|entry| {
            format!(
                "@{}\t{}\t{}",
                entry.id.0,
                entry.browser.as_str(),
                entry.name
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let text = if text.is_empty() {
        text
    } else {
        format!("{text}\n")
    };
    atomic_write(&catalog_path(), text.as_bytes())
}
fn catalog_assignment_available(
    browser: Browser,
    name: &str,
    requested: Option<ProfileId>,
) -> io::Result<()> {
    if !valid_name(name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid profile name",
        ));
    }
    let entries = read_catalog()?;
    if let Some(entry) = entries
        .iter()
        .find(|entry| entry.browser == browser && entry.name == name)
    {
        if requested.is_some_and(|id| id != entry.id) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "profile already has a different ID",
            ));
        }
    }
    if requested.is_some_and(|id| {
        entries
            .iter()
            .any(|entry| entry.id == id && !(entry.browser == browser && entry.name == name))
    }) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "profile ID is already assigned",
        ));
    }
    Ok(())
}
fn catalog_id(browser: Browser, name: &str, requested: Option<ProfileId>) -> io::Result<ProfileId> {
    if !valid_name(name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid profile name",
        ));
    }
    let mut entries = read_catalog()?;
    if let Some(entry) = entries
        .iter()
        .find(|entry| entry.browser == browser && entry.name == name)
    {
        if requested.is_some_and(|id| id != entry.id) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "profile already has a different ID",
            ));
        }
        return Ok(entry.id);
    }
    let id = match requested {
        Some(id) => id,
        None => {
            let mut id = 1;
            while entries.iter().any(|entry| entry.id.0 == id) {
                id = id.checked_add(1).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::AlreadyExists, "no profile IDs available")
                })?;
            }
            ProfileId(id)
        }
    };
    if entries.iter().any(|entry| entry.id == id) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "profile ID is already assigned",
        ));
    }
    entries.push(CatalogEntry {
        id,
        browser,
        name: name.into(),
    });
    write_catalog(&entries)?;
    Ok(id)
}
fn catalog_remove(browser: Browser, name: &str) -> io::Result<()> {
    let mut entries = read_catalog()?;
    entries.retain(|entry| entry.browser != browser || entry.name != name);
    write_catalog(&entries)
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
        let Some((_, path)) = &discovered else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "profile is not registered in profiles.ini",
            ));
        };
        if !same_path(&owned, path) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "profile name collision",
            ));
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
fn open_tty() -> io::Result<fs::File> {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "no controlling terminal for prompt",
            )
        })
}
fn require_tty() -> io::Result<()> {
    open_tty().map(|_| ())
}
fn prompt(message: &str) -> io::Result<String> {
    let mut tty = open_tty()?;
    eprint!("{message}");
    io::stderr().flush()?;
    let mut answer = String::new();
    use std::io::BufRead;
    io::BufReader::new(&mut tty).read_line(&mut answer)?;
    Ok(answer.trim_end_matches(['\r', '\n']).to_owned())
}
fn prompt_browser() -> io::Result<Browser> {
    match prompt("Browser [firefox (default)|librewolf]: ")?.as_str() {
        "" | "firefox" => Ok(Browser::Firefox),
        "librewolf" => Ok(Browser::Librewolf),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "browser must be firefox or librewolf",
        )),
    }
}
fn confirm_unmanaged(operation: &str, name: &str, dir: &Path) -> io::Result<bool> {
    Ok(prompt(&format!(
        "{operation} unmanaged registered profile {name} at {}? [y|N] ",
        dir.display()
    ))? == "y")
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

fn create_locked(
    browser: Browser,
    name: &str,
    template: &[u8],
    make_default: bool,
) -> io::Result<()> {
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
        write_user_js(&dir, template, false)?;
        // The marker is the last profile-local step, before the registry can
        // change. Any later failure removes the whole new directory.
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(MARKER))?;
        if make_default {
            set_default_locked(browser, name)?;
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
    Ok(profiles_from_ini(browser)?
        .into_iter()
        .filter(|(name, path)| all || is_owned_profile(browser, name, path))
        .map(|(name, _)| name)
        .collect())
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
fn remove_locked(browser: Browser, name: &str) -> io::Result<()> {
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
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    unregister_profile_locked(browser, name, &dir)?;
    // Keep the ID reserved if this write fails after deletion.
    catalog_remove(browser, name)
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
fn apply_locked(browser: Browser, name: &str, template: &[u8], backup: bool) -> io::Result<()> {
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
    }
    write_user_js(&dir, template, backup)
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
fn resolve_selector(
    browser: Option<Browser>,
    selector: Selector,
    allow_default: bool,
) -> io::Result<(Browser, String)> {
    match selector {
        Selector::Id(id) => {
            let entry = read_catalog()?
                .into_iter()
                .find(|entry| entry.id == id)
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown profile ID"))?;
            if browser.is_some_and(|browser| browser != entry.browser) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "profile ID belongs to a different browser",
                ));
            }
            let path = resolve(entry.browser, &entry.name)?;
            safe_dir(&path)?;
            if !path.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "profile does not exist",
                ));
            }
            Ok((entry.browser, entry.name))
        }
        Selector::Name(name) => {
            let browser = browser.map(Ok).unwrap_or_else(prompt_browser)?;
            let resolved_name = if name == "default" && allow_default {
                default_profile(browser)?
                    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no default profile"))?
                    .0
            } else if name == "default" {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "default is only valid for apply or launch",
                ));
            } else {
                name
            };
            let path = resolve(browser, &resolved_name)?;
            safe_dir(&path)?;
            if !path.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "profile does not exist",
                ));
            }
            Ok((browser, resolved_name))
        }
    }
}
#[derive(Clone, Copy)]
enum Mutation {
    Apply,
    Default,
    Remove { yes: bool },
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Confirmation {
    None,
    Unmanaged,
    Default,
    Remove,
}
#[derive(PartialEq, Eq)]
struct MutationTarget {
    browser: Browser,
    name: String,
    path: PathBuf,
    owned: bool,
    default: bool,
    confirmation: Confirmation,
}
fn mutation_target(browser: Browser, name: &str, mutation: Mutation) -> io::Result<MutationTarget> {
    let path = resolve(browser, name)?;
    safe_dir(&path)?;
    if !path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "profile does not exist",
        ));
    }
    let owned = is_owned_profile(browser, name, &path);
    let default =
        default_profile(browser)?.is_some_and(|(_, default_path)| same_path(&default_path, &path));
    let confirmation = match mutation {
        Mutation::Apply
            if profiles_from_ini(browser)?
                .iter()
                .find(|(_, registered_path)| same_path(registered_path, &path))
                .is_some_and(|(registered_name, _)| {
                    !is_owned_profile(browser, registered_name, &path)
                }) =>
        {
            Confirmation::Unmanaged
        }
        Mutation::Default if !owned => Confirmation::Unmanaged,
        Mutation::Remove { .. } if !owned => Confirmation::Unmanaged,
        Mutation::Remove { .. } if default => Confirmation::Default,
        Mutation::Remove { yes: false } => Confirmation::Remove,
        _ => Confirmation::None,
    };
    Ok(MutationTarget {
        browser,
        name: name.into(),
        path,
        owned,
        default,
        confirmation,
    })
}
fn confirm_target(target: &MutationTarget, operation: &str) -> io::Result<bool> {
    match target.confirmation {
        Confirmation::None => Ok(true),
        Confirmation::Unmanaged => confirm_unmanaged(operation, &target.name, &target.path),
        Confirmation::Default => Ok(prompt(&format!(
            "Remove browser default profile {}? [y|N] ",
            target.name
        ))? == "y"),
        Confirmation::Remove => Ok(prompt(&format!("Remove {}? [y/N] ", target.name))? == "y"),
    }
}
fn retry() -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        "profile changed while waiting for lock; retry",
    )
}
fn lock_target(
    target: &MutationTarget,
    mutation: Mutation,
) -> io::Result<(MutationLock, CatalogLock)> {
    let mutation_lock = MutationLock::acquire(target.browser)?;
    let catalog_lock = CatalogLock::acquire()?;
    if mutation_target(target.browser, &target.name, mutation)? != *target {
        return Err(retry());
    }
    Ok((mutation_lock, catalog_lock))
}
fn revalidate_selector(
    browser: Option<Browser>,
    selector: Selector,
    allow_default: bool,
    target: &MutationTarget,
) -> io::Result<()> {
    if resolve_selector(browser, selector, allow_default)? != (target.browser, target.name.clone())
    {
        return Err(retry());
    }
    Ok(())
}
fn create_with_id(
    id: Option<ProfileId>,
    browser: Option<Browser>,
    name: Option<String>,
    default: Option<bool>,
    template_path: Option<String>,
) -> io::Result<()> {
    let template = read_template(template_path.as_deref())?;
    let name = name.unwrap_or(template_name(template_path.as_deref())?);
    let browser = match browser {
        Some(browser) => browser,
        None => prompt_browser()?,
    };
    let make_default = match default {
        Some(default) => default,
        None => prompt("Make default? [N|y] ")? == "y",
    };
    let _mutation_lock = MutationLock::acquire(browser)?;
    let _catalog_lock = CatalogLock::acquire()?;
    catalog_assignment_available(browser, &name, id)?;
    let existing = read_catalog()?
        .iter()
        .any(|entry| entry.browser == browser && entry.name == name);
    let id = catalog_id(browser, &name, id)?;
    if let Err(error) = create_locked(browser, &name, &template, make_default) {
        if !existing {
            if let Err(cleanup) = catalog_remove(browser, &name) {
                return Err(io::Error::new(
                    cleanup.kind(),
                    format!("{error}; catalog rollback failed: {cleanup}"),
                ));
            }
        }
        return Err(error);
    }
    println!("@{}\t{}\t{}", id.0, browser.as_str(), name);
    Ok(())
}
fn list_with_ids(browser: Option<Browser>, all: bool) -> io::Result<()> {
    let browsers = browser
        .map(|browser| vec![browser])
        .unwrap_or_else(|| vec![Browser::Firefox, Browser::Librewolf]);
    let mut rows = Vec::new();
    for browser in browsers {
        for name in list_names(browser, all)? {
            if !valid_name(&name) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unsafe profile name",
                ));
            }
            let path = resolve(browser, &name)?;
            safe_dir(&path)?;
            if !path.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "profile does not exist",
                ));
            }
            rows.push((catalog_id(browser, &name, None)?, browser, name));
        }
    }
    rows.sort_by_key(|(id, _, _)| *id);
    for (id, browser, name) in rows {
        println!("@{}\t{}\t{}", id.0, browser.as_str(), name);
    }
    Ok(())
}
fn prompt_selector_browser(
    browser: Option<Browser>,
    selector: &Selector,
) -> io::Result<Option<Browser>> {
    if browser.is_none() && matches!(selector, Selector::Name(_)) {
        Ok(Some(prompt_browser()?))
    } else {
        Ok(browser)
    }
}
fn launchable_profiles(browser: Option<Browser>) -> io::Result<Vec<CatalogEntry>> {
    let browsers = browser
        .map(|browser| vec![browser])
        .unwrap_or_else(|| vec![Browser::Firefox, Browser::Librewolf]);
    let mut choices = Vec::new();
    for browser in browsers {
        let names = profiles_from_ini(browser)?
            .into_iter()
            .map(|(name, _)| name)
            .collect::<BTreeSet<_>>();
        for name in names {
            if !valid_name(&name) {
                continue;
            }
            let Ok(path) = resolve(browser, &name) else {
                continue;
            };
            if safe_dir(&path).is_ok() && path.is_dir() {
                choices.push(CatalogEntry {
                    id: catalog_id(browser, &name, None)?,
                    browser,
                    name,
                });
            }
        }
    }
    choices.sort_by_key(|choice| choice.id);
    Ok(choices)
}
fn pick_profile(browser: Option<Browser>) -> io::Result<CatalogEntry> {
    let choices = {
        let _catalog_lock = CatalogLock::acquire()?;
        launchable_profiles(browser)?
    };
    if choices.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no launchable profiles",
        ));
    }
    let mut tty = open_tty()?;
    for choice in &choices {
        writeln!(
            tty,
            "@{} {} ({})",
            choice.id.0,
            choice.name,
            choice.browser.as_str()
        )?;
    }
    write!(tty, "Profile ID: ")?;
    tty.flush()?;
    let mut answer = String::new();
    use std::io::BufRead;
    io::BufReader::new(&mut tty).read_line(&mut answer)?;
    let id = ProfileId::parse(answer.trim_end_matches(['\r', '\n']))?;
    choices
        .into_iter()
        .find(|choice| choice.id == id)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid profile ID"))
}
fn launch_from_picker(browser: Option<Browser>, private: bool, args: &[String]) -> io::Result<()> {
    let choice = pick_profile(browser)?;
    let _catalog_lock = CatalogLock::acquire()?;
    let (selected_browser, name) = resolve_selector(browser, Selector::Id(choice.id), true)?;
    if (selected_browser, name.as_str()) != (choice.browser, choice.name.as_str()) {
        return Err(retry());
    }
    launch(selected_browser, &name, private, args)
}
fn run() -> io::Result<()> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.as_slice() == ["help"] {
        println!("{}", cli::HELP);
        return Ok(());
    }
    let command = cli::parse(args)?;
    match command {
        Command::List { browser, all } => {
            let _catalog_lock = CatalogLock::acquire()?;
            list_with_ids(browser, all)
        }
        Command::Create {
            id,
            browser,
            name,
            default,
            template,
        } => create_with_id(id, browser, name, default, template),
        Command::DefaultGet { browser } => {
            let browser = match browser {
                Some(browser) => browser,
                None => prompt_browser()?,
            };
            let _catalog_lock = CatalogLock::acquire()?;
            let (name, _) = default_profile(browser)?
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no default profile"))?;
            let id = catalog_id(browser, &name, None)?;
            println!("@{}\t{}\t{}", id.0, browser.as_str(), name);
            Ok(())
        }
        Command::Apply {
            browser,
            selector,
            template,
            backup,
        } => {
            let template = read_template(template.as_deref())?;
            let browser = prompt_selector_browser(browser, &selector)?;
            let (selected_browser, name) = resolve_selector(browser, selector.clone(), true)?;
            let target = mutation_target(selected_browser, &name, Mutation::Apply)?;
            if !confirm_target(&target, "Apply to")? {
                return Ok(());
            }
            let (_mutation_lock, _catalog_lock) = lock_target(&target, Mutation::Apply)?;
            revalidate_selector(browser, selector, true, &target)?;
            read_catalog()?;
            apply_locked(selected_browser, &name, &template, backup)?;
            catalog_id(selected_browser, &name, None).map(|_| ())
        }
        Command::DefaultSet { browser, selector } => {
            let browser = prompt_selector_browser(browser, &selector)?;
            let (selected_browser, name) = resolve_selector(browser, selector.clone(), false)?;
            let target = mutation_target(selected_browser, &name, Mutation::Default)?;
            if !confirm_target(&target, "Set default for")? {
                return Ok(());
            }
            let (_mutation_lock, _catalog_lock) = lock_target(&target, Mutation::Default)?;
            revalidate_selector(browser, selector, false, &target)?;
            read_catalog()?;
            set_default_locked(selected_browser, &name)?;
            catalog_id(selected_browser, &name, None).map(|_| ())
        }
        Command::Remove {
            browser,
            selector,
            yes,
        } => {
            let browser = prompt_selector_browser(browser, &selector)?;
            let (selected_browser, name) = resolve_selector(browser, selector.clone(), false)?;
            let mutation = Mutation::Remove { yes };
            let target = mutation_target(selected_browser, &name, mutation)?;
            if !confirm_target(&target, "Remove")? {
                return Ok(());
            }
            let (_mutation_lock, _catalog_lock) = lock_target(&target, mutation)?;
            revalidate_selector(browser, selector, false, &target)?;
            read_catalog()?;
            remove_locked(selected_browser, &name)
        }
        Command::Launch {
            browser,
            selector: Some(selector),
            private,
            args,
        } => {
            let browser = prompt_selector_browser(browser, &selector)?;
            let _catalog_lock = CatalogLock::acquire()?;
            let (browser, name) = resolve_selector(browser, selector, true)?;
            catalog_id(browser, &name, None)?;
            launch(browser, &name, private, &args)
        }
        Command::Launch {
            browser,
            selector: None,
            private,
            args,
        } => {
            require_tty()?;
            launch_from_picker(browser, private, &args)
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
