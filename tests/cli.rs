use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
unsafe extern "C" {
    fn setsid() -> i32;
}

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = env::temp_dir().join(format!(
            "browserprofile-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }
    fn data(&self) -> PathBuf {
        self.root.join("data")
    }
    fn config(&self) -> PathBuf {
        self.root.join("config")
    }
    fn home(&self) -> PathBuf {
        self.root.join("home")
    }
    fn run(&self, bin: &str, args: &[&str]) -> Output {
        Command::new(bin)
            .args(args)
            .env("XDG_DATA_HOME", self.data())
            .env("XDG_CONFIG_HOME", self.config())
            .env("HOME", self.home())
            .output()
            .unwrap()
    }
    fn catalog(&self) -> PathBuf {
        self.data().join("browserprofile/profiles.catalog")
    }
    #[cfg(unix)]
    fn run_without_tty(&self, bin: &str, args: &[&str]) -> Output {
        use std::os::unix::process::CommandExt;

        let mut command = Command::new(bin);
        command
            .args(args)
            .env("XDG_DATA_HOME", self.data())
            .env("XDG_CONFIG_HOME", self.config())
            .env("HOME", self.home());
        // SAFETY: the child runs setsid before exec, so it has no controlling terminal.
        unsafe {
            command.pre_exec(|| {
                // SAFETY: setsid has no Rust-side preconditions in the child process.
                if setsid() == -1 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        command.output().unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn bp() -> &'static str {
    env!("CARGO_BIN_EXE_bp")
}
fn text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn ids_are_global_stable_and_aliases_match() {
    let fixture = Fixture::new();
    for (args, expected) in [
        (
            &[
                "create",
                "@0",
                "--name",
                "zero",
                "--browser",
                "firefox",
                "--default",
                "no",
            ][..],
            "@0\tfirefox\tzero\n",
        ),
        (
            &[
                "create",
                "--name",
                "one",
                "--browser",
                "firefox",
                "--default",
                "no",
            ][..],
            "@1\tfirefox\tone\n",
        ),
        (
            &[
                "create",
                "--name",
                "one",
                "--browser",
                "librewolf",
                "--default",
                "no",
            ][..],
            "@2\tlibrewolf\tone\n",
        ),
    ] {
        let output = fixture.run(bp(), args);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(text(&output), expected);
    }
    assert!(!fixture
        .run(
            bp(),
            &[
                "create",
                "@1",
                "--name",
                "collision",
                "--browser",
                "librewolf",
                "--default",
                "no"
            ],
        )
        .status
        .success());
    let listed = fixture.run(bp(), &["list"]);
    assert!(listed.status.success(), "{listed:?}");
    assert_eq!(
        text(&listed),
        "@0\tfirefox\tzero\n@1\tfirefox\tone\n@2\tlibrewolf\tone\n"
    );
    assert_eq!(
        text(&fixture.run(env!("CARGO_BIN_EXE_browserprofile"), &["list"])),
        text(&listed)
    );
    let mismatch = fixture.run(bp(), &["apply", "@0", "--browser", "librewolf"]);
    assert_eq!(mismatch.status.code(), Some(2));
}

#[test]
fn hash_ids_are_rejected_and_numeric_names_remain_names() {
    let fixture = Fixture::new();
    let output = fixture.run(
        bp(),
        &[
            "create",
            "--name",
            "123",
            "--browser",
            "firefox",
            "--default",
            "no",
        ],
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(text(&output), "@1\tfirefox\t123\n");
    assert!(fixture
        .run(bp(), &["apply", "123", "--browser", "firefox"])
        .status
        .success());
    assert_eq!(
        fixture
            .run(bp(), &["apply", "#1", "--browser", "firefox"])
            .status
            .code(),
        Some(2)
    );
}

#[test]
fn legacy_profiles_receive_stable_ids_without_content_changes() {
    let fixture = Fixture::new();
    for browser in ["firefox", "librewolf"] {
        let profile = fixture
            .data()
            .join(format!("browserprofile/{browser}/legacy"));
        fs::create_dir_all(&profile).unwrap();
        fs::write(profile.join(".browserprofile-owned"), "").unwrap();
        fs::write(profile.join("keep"), browser).unwrap();
    }

    let first = fixture.run(bp(), &["list"]);
    assert!(first.status.success(), "{first:?}");
    assert_eq!(text(&first), "@1\tfirefox\tlegacy\n@2\tlibrewolf\tlegacy\n");
    assert_eq!(text(&fixture.run(bp(), &["list"])), text(&first));
    assert_eq!(
        fs::read_to_string(fixture.data().join("browserprofile/firefox/legacy/keep")).unwrap(),
        "firefox"
    );
    assert_eq!(
        fs::read_to_string(fixture.data().join("browserprofile/librewolf/legacy/keep")).unwrap(),
        "librewolf"
    );
}

#[test]
fn legacy_librewolf_registry_is_reconciled_before_global_ids() {
    let fixture = Fixture::new();
    let profile = fixture.data().join("browserprofile/librewolf/legacy");
    fs::create_dir_all(&profile).unwrap();
    fs::write(profile.join(".browserprofile-owned"), "").unwrap();
    fs::write(profile.join("keep"), "unchanged").unwrap();
    let legacy_ini = fixture.config().join("librewolf/librewolf/profiles.ini");
    fs::create_dir_all(legacy_ini.parent().unwrap()).unwrap();
    fs::write(
        legacy_ini,
        format!(
            "[Profile0]\nName=legacy\nIsRelative=0\nPath={}\nDefault=0\n",
            profile.display()
        ),
    )
    .unwrap();

    let output = fixture.run(bp(), &["list"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(text(&output), "@1\tlibrewolf\tlegacy\n");
    assert_eq!(
        fs::read_to_string(fixture.catalog()).unwrap(),
        "@1\tlibrewolf\tlegacy\n"
    );
    assert!(
        fs::read_to_string(fixture.home().join(".librewolf/profiles.ini"))
            .unwrap()
            .contains("Name=legacy")
    );
    assert_eq!(
        fs::read_to_string(profile.join("keep")).unwrap(),
        "unchanged"
    );
}

#[test]
fn inferred_librewolf_id_reconciles_legacy_registry() {
    let fixture = Fixture::new();
    let profile = fixture.data().join("browserprofile/librewolf/inferred");
    fs::create_dir_all(&profile).unwrap();
    fs::write(profile.join(".browserprofile-owned"), "").unwrap();
    let legacy_ini = fixture.config().join("librewolf/librewolf/profiles.ini");
    fs::create_dir_all(legacy_ini.parent().unwrap()).unwrap();
    fs::write(
        legacy_ini,
        format!(
            "[Profile0]\nName=inferred\nIsRelative=0\nPath={}\nDefault=0\n",
            profile.display()
        ),
    )
    .unwrap();
    fs::create_dir_all(fixture.catalog().parent().unwrap()).unwrap();
    fs::write(fixture.catalog(), "@1\tlibrewolf\tinferred\n").unwrap();

    let output = fixture.run(bp(), &["apply", "@1"]);
    assert!(output.status.success(), "{output:?}");
    assert!(profile.join("user.js").is_file());
    assert!(
        fs::read_to_string(fixture.home().join(".librewolf/profiles.ini"))
            .unwrap()
            .contains("Name=inferred")
    );
}

#[cfg(unix)]
#[test]
fn no_tty_prevents_librewolf_reconciliation_before_prompts() {
    let fixture = Fixture::new();
    let profile = fixture.data().join("browserprofile/librewolf/legacy");
    fs::create_dir_all(&profile).unwrap();
    fs::write(profile.join(".browserprofile-owned"), "").unwrap();
    let legacy_ini = fixture.config().join("librewolf/librewolf/profiles.ini");
    fs::create_dir_all(legacy_ini.parent().unwrap()).unwrap();
    fs::write(
        legacy_ini,
        format!(
            "[Profile0]\nName=legacy\nIsRelative=0\nPath={}\nDefault=0\n",
            profile.display()
        ),
    )
    .unwrap();

    assert!(!fixture.run_without_tty(bp(), &["launch"]).status.success());
    assert!(!fixture
        .run_without_tty(bp(), &["create", "--name", "new", "--browser", "librewolf"])
        .status
        .success());
    assert!(!fixture.home().join(".librewolf/profiles.ini").exists());

    let unmanaged = fixture.root.join("unmanaged");
    fs::create_dir_all(&unmanaged).unwrap();
    let active_ini = fixture.home().join(".librewolf/profiles.ini");
    fs::create_dir_all(active_ini.parent().unwrap()).unwrap();
    fs::write(
        &active_ini,
        format!(
            "[Profile0]\nName=unmanaged\nIsRelative=0\nPath={}\nDefault=0\n",
            unmanaged.display()
        ),
    )
    .unwrap();
    assert!(!fixture
        .run_without_tty(bp(), &["apply", "unmanaged", "--browser", "librewolf"])
        .status
        .success());
    assert!(!fs::read_to_string(active_ini)
        .unwrap()
        .contains("Name=legacy"));
}

#[test]
fn explicit_firefox_operation_skips_librewolf_reconciliation() {
    let fixture = Fixture::new();
    let legacy_ini = fixture.config().join("librewolf/librewolf/profiles.ini");
    fs::create_dir_all(legacy_ini.parent().unwrap()).unwrap();
    fs::write(
        legacy_ini,
        "[Profile0]\nName=one\nName=two\nIsRelative=0\nPath=/tmp/one\n",
    )
    .unwrap();

    let output = fixture.run(
        bp(),
        &[
            "create",
            "--name",
            "firefox-only",
            "--browser",
            "firefox",
            "--default",
            "no",
        ],
    );
    assert!(output.status.success(), "{output:?}");
    assert!(!fixture.home().join(".librewolf/profiles.ini").exists());
}

#[test]
fn failed_create_rolls_back_and_invalid_catalog_blocks_name_removal() {
    let fixture = Fixture::new();
    let ini = fixture.config().join("mozilla/firefox/profiles.ini");
    fs::create_dir_all(ini.parent().unwrap()).unwrap();
    fs::write(
        &ini,
        "[Profile0]\nName=exists\nIsRelative=0\nPath=/tmp/exists\n",
    )
    .unwrap();
    assert!(!fixture
        .run(
            bp(),
            &[
                "create",
                "@9",
                "--name",
                "exists",
                "--browser",
                "firefox",
                "--default",
                "no"
            ],
        )
        .status
        .success());
    assert!(!fs::read_to_string(fixture.catalog())
        .unwrap_or_default()
        .contains("@9"));

    let created = fixture.run(
        bp(),
        &[
            "create",
            "--name",
            "remove-me",
            "--browser",
            "firefox",
            "--default",
            "no",
        ],
    );
    assert!(created.status.success(), "{created:?}");
    fs::write(fixture.catalog(), "not a catalog\n").unwrap();
    assert!(!fixture
        .run(
            bp(),
            &["remove", "remove-me", "--browser", "firefox", "--yes"]
        )
        .status
        .success());
    assert!(fixture
        .data()
        .join("browserprofile/firefox/remove-me")
        .is_dir());
}

#[test]
fn no_tty_prompt_does_not_create_catalog_state() {
    let fixture = Fixture::new();
    let profile = fixture.root.join("unmanaged");
    fs::create_dir_all(&profile).unwrap();
    let ini = fixture.config().join("mozilla/firefox/profiles.ini");
    fs::create_dir_all(ini.parent().unwrap()).unwrap();
    fs::write(
        ini,
        format!(
            "[Profile0]\nName=unmanaged\nIsRelative=0\nPath={}\n",
            profile.display()
        ),
    )
    .unwrap();
    let output = fixture.run_without_tty(bp(), &["remove", "unmanaged", "--browser", "firefox"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("no controlling terminal"));
    assert!(profile.is_dir());
    assert!(!fixture.data().join("browserprofile").exists());
}

#[cfg(unix)]
#[test]
fn bare_launch_without_a_tty_does_not_launch() {
    let fixture = Fixture::new();
    assert!(fixture
        .run(
            bp(),
            &[
                "create",
                "--name",
                "launchable",
                "--browser",
                "firefox",
                "--default",
                "no",
            ],
        )
        .status
        .success());
    let output = fixture.run_without_tty(bp(), &["launch"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("no controlling terminal"));
}

#[test]
fn id_mutations_do_not_require_a_tty_when_managed() {
    let fixture = Fixture::new();
    for args in [
        &[
            "create",
            "@0",
            "--name",
            "apply",
            "--browser",
            "firefox",
            "--default",
            "no",
        ][..],
        &[
            "create",
            "@1",
            "--name",
            "set-default",
            "--browser",
            "firefox",
            "--default",
            "no",
        ][..],
        &[
            "create",
            "@2",
            "--name",
            "remove",
            "--browser",
            "firefox",
            "--default",
            "no",
        ][..],
    ] {
        assert!(fixture.run(bp(), args).status.success());
    }
    assert!(fixture.run(bp(), &["apply", "@0"]).status.success());
    assert!(fixture
        .data()
        .join("browserprofile/firefox/apply/user.js")
        .is_file());
    assert!(fixture
        .run(bp(), &["default", "set", "@1"])
        .status
        .success());
    assert!(
        fs::read_to_string(fixture.config().join("mozilla/firefox/profiles.ini"))
            .unwrap()
            .contains("Name=set-default\nIsRelative=0")
    );
    assert!(fixture
        .run(bp(), &["remove", "@2", "--yes"])
        .status
        .success());
    assert!(!fixture
        .data()
        .join("browserprofile/firefox/remove")
        .exists());
}

#[cfg(unix)]
#[test]
fn launch_passes_literal_arguments_to_fake_browser() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new();
    let created = fixture.run(
        bp(),
        &[
            "create",
            "@0",
            "--name",
            "launch",
            "--browser",
            "firefox",
            "--default",
            "no",
        ],
    );
    assert!(created.status.success(), "{created:?}");
    let fake = fixture.root.join("bin");
    fs::create_dir_all(&fake).unwrap();
    let script = fake.join("firefox");
    fs::write(
        &script,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$FAKE_ARGS\"\n",
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let args_file = fixture.root.join("args");
    let output = Command::new(bp())
        .args(["launch", "@0", "--", "x;touch-not-run"])
        .env("XDG_DATA_HOME", fixture.data())
        .env("XDG_CONFIG_HOME", fixture.config())
        .env("HOME", fixture.home())
        .env("FAKE_ARGS", &args_file)
        .env("PATH", fake)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let deadline = Instant::now() + Duration::from_secs(2);
    while !args_file.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let args = fs::read_to_string(args_file).unwrap();
    assert!(args.contains("x;touch-not-run\n"));
}
