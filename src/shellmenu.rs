//! Shell integration — user-level right-click / file-manager context-menu
//! entries that launch `mlo --open <selection>` on Windows, macOS and Linux.
//!
//! Everything here installs into the *current user's* profile only (HKCU,
//! `~/Library/Services`, `~/.local/share`); it never needs administrator rights.
//!
//! All writers are idempotent and go through [`crate::atomic`]; every process
//! spawn (`reg.exe`, `lsregister`, `update-desktop-database`) captures its
//! output and turns a failure to start into a named [`MloError`] instead of a
//! panic. The pure builders (`desktop_entry`, `document_wflow`, `add_argv`, …)
//! contain no I/O and are exercised by the tests on every platform.

use crate::atomic;
use crate::error::{IoResultExt, MloError, Result};
use std::path::{Path, PathBuf};
use std::process::Output;
use tracing::{debug, info, warn};

pub struct ShellMenuItem {
    pub os: &'static str,
    pub name: String,
    pub detail: String,
    pub installed: bool,
}

impl ShellMenuItem {
    fn new(os: &'static str, name: impl Into<String>, detail: impl Into<String>, installed: bool) -> Self {
        Self { os, name: name.into(), detail: detail.into(), installed }
    }
}

/// True on the three platforms we can integrate with.
pub fn supported() -> bool {
    cfg!(windows) || cfg!(target_os = "macos") || cfg!(target_os = "linux")
}

/// Describe the context-menu entries for `exe` and whether each is present.
pub fn status(exe: &Path) -> Vec<ShellMenuItem> {
    if cfg!(windows) {
        windows::status(exe)
    } else if cfg!(target_os = "macos") {
        macos::status(exe)
    } else if cfg!(target_os = "linux") {
        linux::status(exe)
    } else {
        Vec::new()
    }
}

/// Install the context-menu entries for `exe`, returning human-readable actions.
pub fn install(exe: &Path) -> Result<Vec<String>> {
    if exe.as_os_str().is_empty() {
        return Err(MloError::Invalid("shell integration needs a non-empty executable path".into()));
    }
    if cfg!(windows) {
        windows::install(exe)
    } else if cfg!(target_os = "macos") {
        macos::install(exe)
    } else if cfg!(target_os = "linux") {
        linux::install(exe)
    } else {
        Err(MloError::Invalid(format!(
            "shell integration unsupported on {}",
            std::env::consts::OS
        )))
    }
}

/// Remove every entry this module can have installed, returning actions taken.
pub fn uninstall() -> Result<Vec<String>> {
    if cfg!(windows) {
        windows::uninstall()
    } else if cfg!(target_os = "macos") {
        macos::uninstall()
    } else if cfg!(target_os = "linux") {
        linux::uninstall()
    } else {
        Err(MloError::Invalid(format!(
            "shell integration unsupported on {}",
            std::env::consts::OS
        )))
    }
}

// --- shared process helpers -------------------------------------------------

/// Run a program, capturing stdout/stderr. A failure to *start* is a named
/// (`TOOL_UNAVAILABLE`) error.
fn run_capture(program: &str, args: &[String]) -> Result<Output> {
    std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|e| MloError::tool(program, format!("cannot spawn: {e}")))
}

/// Like [`run_capture`] but a missing binary is reported as `None` rather than
/// an error ("if present" semantics).
fn run_optional(program: &str, args: &[String]) -> Result<Option<Output>> {
    match std::process::Command::new(program).args(args).output() {
        Ok(out) => Ok(Some(out)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(MloError::tool(program, format!("cannot spawn: {e}"))),
    }
}

fn require_success(program: &str, out: &Output) -> Result<()> {
    if out.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(MloError::tool(
            program,
            format!("exit {:?}: {}", out.status.code(), stderr.trim()),
        ))
    }
}

fn home_dir() -> Result<PathBuf> {
    directories::BaseDirs::new()
        .map(|b| b.home_dir().to_path_buf())
        .ok_or_else(|| MloError::NotFound("home directory for shell integration".into()))
}

// --- Windows ----------------------------------------------------------------

mod windows {
    use super::*;

    pub(super) const DIR_KEY: &str = r"HKCU\Software\Classes\Directory\shell\mlo";
    pub(super) const FILE_KEY: &str = r"HKCU\Software\Classes\*\shell\mlo";
    pub(super) const BG_KEY: &str = r"HKCU\Software\Classes\Directory\Background\shell\mlo";
    pub(super) const APP_PATHS_KEY: &str =
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\App Paths\mlo.exe";

    /// One `reg add` operation: a key, an optional named value (`None` = default
    /// value, i.e. `/ve`) and its data.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(super) struct RegEntry {
        pub key: String,
        pub name: Option<String>,
        pub data: String,
    }

    fn menu_label() -> &'static str {
        "Open in mlo"
    }

    fn command_for(exe: &Path, placeholder: &str) -> String {
        format!("\"{}\" --open \"{}\"", exe.display(), placeholder)
    }

    /// Pure builder: every registry value the installer writes.
    pub(super) fn install_entries(exe: &Path) -> Vec<RegEntry> {
        let exe_s = exe.display().to_string();
        vec![
            RegEntry { key: DIR_KEY.into(), name: None, data: menu_label().into() },
            RegEntry {
                key: format!(r"{DIR_KEY}\command"),
                name: None,
                data: command_for(exe, "%1"),
            },
            RegEntry { key: FILE_KEY.into(), name: None, data: menu_label().into() },
            RegEntry {
                key: format!(r"{FILE_KEY}\command"),
                name: None,
                data: command_for(exe, "%1"),
            },
            RegEntry { key: BG_KEY.into(), name: None, data: menu_label().into() },
            RegEntry {
                key: format!(r"{BG_KEY}\command"),
                name: None,
                data: command_for(exe, "%V"),
            },
            RegEntry { key: APP_PATHS_KEY.into(), name: None, data: exe_s },
        ]
    }

    /// Pure builder: the keys uninstall removes (recursively).
    pub(super) fn delete_keys() -> Vec<String> {
        vec![DIR_KEY.into(), FILE_KEY.into(), BG_KEY.into(), APP_PATHS_KEY.into()]
    }

    /// Pure builder: the argv handed to `reg.exe` to add `entry`.
    pub(super) fn add_argv(entry: &RegEntry) -> Vec<String> {
        let mut argv = vec!["add".to_string(), entry.key.clone()];
        match &entry.name {
            None => argv.push("/ve".into()),
            Some(name) => {
                argv.push("/v".into());
                argv.push(name.clone());
            }
        }
        argv.push("/d".into());
        argv.push(entry.data.clone());
        argv.push("/f".into());
        argv
    }

    /// Pure builder: the argv handed to `reg.exe` to remove `key`.
    pub(super) fn delete_argv(key: &str) -> Vec<String> {
        vec!["delete".into(), key.into(), "/f".into()]
    }

    fn query_installed(key: &str) -> bool {
        match run_capture("reg", &["query".into(), key.into()]) {
            Ok(out) => out.status.success(),
            Err(_) => false,
        }
    }

    pub(super) fn status(_exe: &Path) -> Vec<ShellMenuItem> {
        let items = [
            ("Folder context menu", DIR_KEY),
            ("File context menu", FILE_KEY),
            ("Folder background menu", BG_KEY),
            ("App Paths entry", APP_PATHS_KEY),
        ];
        items
            .into_iter()
            .map(|(name, key)| ShellMenuItem::new("windows", name, key, query_installed(key)))
            .collect()
    }

    pub(super) fn install(exe: &Path) -> Result<Vec<String>> {
        let mut actions = Vec::new();
        for entry in install_entries(exe) {
            let argv = add_argv(&entry);
            let out = run_capture("reg", &argv)?;
            require_success("reg", &out)?;
            actions.push(format!("set registry value {} (default) = {}", entry.key, entry.data));
        }
        info!(target: "shellmenu", entries = actions.len(), "windows shell menu installed");
        Ok(actions)
    }

    pub(super) fn uninstall() -> Result<Vec<String>> {
        let mut actions = Vec::new();
        for key in delete_keys() {
            let argv = delete_argv(&key);
            let out = run_capture("reg", &argv)?;
            if out.status.success() {
                actions.push(format!("deleted registry key {key}"));
            } else {
                actions.push(format!("registry key {key} was not present"));
            }
        }
        info!(target: "shellmenu", entries = actions.len(), "windows shell menu uninstalled");
        Ok(actions)
    }
}

// --- macOS ------------------------------------------------------------------

mod macos {
    use super::*;

    pub(super) const BUNDLE_NAME: &str = "mlo.workflow";
    const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

    pub(super) fn bundle_dir(home: &Path) -> PathBuf {
        home.join("Library").join("Services").join(BUNDLE_NAME)
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    /// The shell snippet the Quick Action runs for the selected item.
    pub(super) fn script(exe: &Path) -> String {
        format!("#!/bin/sh\n\"{}\" --open \"$1\"\n", exe.display())
    }

    /// Pure builder: `Contents/Info.plist`.
    pub(super) fn info_plist() -> String {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleIdentifier</key>
	<string>dev.mlo.shellmenu</string>
	<key>CFBundleName</key>
	<string>mlo</string>
	<key>CFBundlePackageType</key>
	<string>BNDL</string>
	<key>CFBundleShortVersionString</key>
	<string>1.0</string>
	<key>CFBundleVersion</key>
	<string>1</string>
</dict>
</plist>
"#
        .to_string()
    }

    /// Pure builder: `Contents/document.wflow` — an Automator "Run Shell Script"
    /// service accepting files and folders (`public.item`).
    pub(super) fn document_wflow(exe: &Path) -> String {
        let script = xml_escape(&script(exe));
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>AMApplicationBuild</key>
	<string>521</string>
	<key>AMApplicationVersion</key>
	<string>2.10</string>
	<key>AMDocumentVersion</key>
	<string>2</string>
	<key>actions</key>
	<array>
		<dict>
			<key>action</key>
			<dict>
				<key>AMAccepts</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Optional</key>
					<false/>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.string</string>
					</array>
				</dict>
				<key>AMActionVersion</key>
				<string>2.0.3</string>
				<key>AMApplication</key>
				<array>
					<string>Automator</string>
				</array>
				<key>AMParameterProperties</key>
				<dict>
					<key>COMMAND_STRING</key>
					<dict/>
					<key>CheckedForUserDefaultShell</key>
					<dict/>
					<key>inputMethod</key>
					<dict/>
					<key>shell</key>
					<dict/>
					<key>source</key>
					<dict/>
				</dict>
				<key>AMProvides</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.string</string>
					</array>
				</dict>
				<key>ActionBundlePath</key>
				<string>/System/Library/Automator/Run Shell Script.action</string>
				<key>ActionName</key>
				<string>Run Shell Script</string>
				<key>ActionParameters</key>
				<dict>
					<key>COMMAND_STRING</key>
					<string>{script}</string>
					<key>CheckedForUserDefaultShell</key>
					<true/>
					<key>inputMethod</key>
					<integer>0</integer>
					<key>shell</key>
					<string>/bin/sh</string>
					<key>source</key>
					<string></string>
				</dict>
				<key>BundleIdentifier</key>
				<string>com.apple.RunShellScript</string>
				<key>CFBundleVersion</key>
				<string>2.0.3</string>
				<key>CanShowSelectedItemsWhenRun</key>
				<false/>
				<key>CanShowWhenRun</key>
				<true/>
				<key>Category</key>
				<array>
					<string>AMCategoryUtilities</string>
				</array>
				<key>Class Name</key>
				<string>RunShellScriptAction</string>
				<key>InputUUID</key>
				<string>9C4E1B0A-2F3D-4E1A-8B7C-0D1E2F3A4B5C</string>
				<key>OutputUUID</key>
				<string>1A2B3C4D-5E6F-4708-9A0B-1C2D3E4F5061</string>
				<key>UUID</key>
				<string>6F5E4D3C-2B1A-4098-8776-655443322110</string>
			</dict>
			<key>isViewVisible</key>
			<integer>1</integer>
		</dict>
	</array>
	<key>connectors</key>
	<dict/>
	<key>workflowMetaData</key>
	<dict>
		<key>serviceInputTypeIdentifier</key>
		<string>com.apple.Automator.fileSystemObject</string>
		<key>serviceOutputTypeIdentifier</key>
		<string>com.apple.Automator.nothing</string>
		<key>serviceProcessesInput</key>
		<integer>0</integer>
		<key>workflowTypeIdentifier</key>
		<string>com.apple.Automator.servicesMenu</string>
	</dict>
</dict>
</plist>
"#
        )
    }

    fn lsregister_argv(bundle: &Path) -> Vec<String> {
        vec!["-f".into(), bundle.display().to_string()]
    }

    pub(super) fn status(_exe: &Path) -> Vec<ShellMenuItem> {
        let bundle = match home_dir() {
            Ok(home) => bundle_dir(&home),
            Err(_) => return Vec::new(),
        };
        let installed = bundle.is_dir();
        vec![ShellMenuItem::new(
            "macos",
            "Finder Quick Action",
            bundle.display().to_string(),
            installed,
        )]
    }

    pub(super) fn install(exe: &Path) -> Result<Vec<String>> {
        let home = home_dir()?;
        let bundle = bundle_dir(&home);
        let contents = bundle.join("Contents");
        atomic::write_atomic_str(contents.join("Info.plist"), &info_plist())?;
        atomic::write_atomic_str(contents.join("document.wflow"), &document_wflow(exe))?;

        let mut actions = vec![
            format!("wrote {}", contents.join("Info.plist").display()),
            format!("wrote {}", contents.join("document.wflow").display()),
        ];

        match run_capture(LSREGISTER, &lsregister_argv(&bundle)) {
            Ok(out) if out.status.success() => {
                actions.push(format!("registered {} with lsregister", bundle.display()));
            }
            Ok(out) => {
                warn!("lsregister exited {:?}", out.status.code());
                actions.push(format!(
                    "note: lsregister exited {:?}; the service may need a re-login to appear",
                    out.status.code()
                ));
            }
            Err(e) => {
                warn!("lsregister unavailable: {e}");
                actions.push(format!("note: could not run lsregister ({e}); Finder may pick it up later"));
            }
        }
        info!(bundle = %bundle.display(), "macos quick action installed");
        Ok(actions)
    }

    pub(super) fn uninstall() -> Result<Vec<String>> {
        let home = home_dir()?;
        let bundle = bundle_dir(&home);
        let mut actions = Vec::new();
        if bundle.is_dir() {
            std::fs::remove_dir_all(&bundle).at(&bundle)?;
            actions.push(format!("removed {}", bundle.display()));
        } else {
            actions.push(format!("{} was not present", bundle.display()));
        }
        match run_capture(LSREGISTER, &lsregister_argv(&bundle)) {
            Ok(out) if out.status.success() => {
                actions.push(format!("refreshed LaunchServices for {}", bundle.display()));
            }
            Ok(out) => {
                warn!("lsregister exited {:?}", out.status.code());
                actions.push(format!("note: lsregister exited {:?}", out.status.code()));
            }
            Err(e) => {
                warn!("lsregister unavailable: {e}");
                actions.push(format!("note: could not run lsregister ({e})"));
            }
        }
        info!("macos quick action uninstalled");
        Ok(actions)
    }
}

// --- Linux ------------------------------------------------------------------

mod linux {
    use super::*;

    pub(super) fn desktop_file(home: &Path) -> PathBuf {
        home.join(".local/share/applications/mlo.desktop")
    }

    pub(super) fn nautilus_script(home: &Path) -> PathBuf {
        home.join(".local/share/nautilus/scripts/Open in mlo")
    }

    pub(super) fn applications_dir(home: &Path) -> PathBuf {
        home.join(".local/share/applications")
    }

    /// Quote a path for a freedesktop `Exec=` field when it contains characters
    /// that would otherwise be split or expanded.
    pub(super) fn desktop_quote(path: &str) -> String {
        let needs = path
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | '$' | '`' | ';'));
        if needs {
            format!("\"{}\"", path.replace('\\', "\\\\").replace('"', "\\\""))
        } else {
            path.to_string()
        }
    }

    /// Quote a path for a POSIX `sh` command word.
    pub(super) fn shell_quote(path: &str) -> String {
        if path
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | '$' | '`' | '&' | '|' | ';' | '<' | '>' | '(' | ')'))
        {
            format!("'{}'", path.replace('\'', "'\\''"))
        } else {
            path.to_string()
        }
    }

    /// Pure builder: the `~/.local/share/applications/mlo.desktop` body.
    pub(super) fn desktop_entry(exe: &Path) -> String {
        format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=mlo\n\
             Exec={} --open %F\n\
             MimeType=inode/directory;audio/flac;audio/mpeg;audio/ogg;audio/mp4;audio/x-wav;video/x-matroska;\n\
             Terminal=true\n",
            desktop_quote(&exe.display().to_string())
        )
    }

    /// Pure builder: the Nautilus script body.
    pub(super) fn nautilus_body(exe: &Path) -> String {
        format!(
            "#!/bin/sh\n\
             # Open the current Nautilus selection in mlo.\n\
             exec {} --open \"$NAUTILUS_SCRIPT_SELECTED_FILE_PATHS\"\n",
            shell_quote(&exe.display().to_string())
        )
    }

    #[cfg(unix)]
    fn set_executable(path: &Path) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path).at(path)?.permissions();
        perms.set_mode(perms.mode() | 0o755);
        std::fs::set_permissions(path, perms).at(path)
    }

    #[cfg(not(unix))]
    fn set_executable(_path: &Path) -> Result<()> {
        Ok(())
    }

    fn refresh_database(home: &Path, actions: &mut Vec<String>) -> Result<()> {
        let dir = applications_dir(home);
        match run_optional("update-desktop-database", &[dir.display().to_string()]) {
            Ok(Some(out)) if out.status.success() => {
                actions.push(format!("ran update-desktop-database {}", dir.display()));
            }
            Ok(Some(out)) => {
                actions.push(format!(
                    "note: update-desktop-database exited {:?}",
                    out.status.code()
                ));
            }
            Ok(None) => {
                actions.push("note: update-desktop-database not present; menu refreshes on next login".into());
            }
            Err(e) => return Err(e),
        }
        Ok(())
    }

    pub(super) fn status(exe: &Path) -> Vec<ShellMenuItem> {
        let home = match home_dir() {
            Ok(h) => h,
            Err(_) => return Vec::new(),
        };
        let _ = exe;
        vec![
            ShellMenuItem::new(
                "linux",
                "Desktop entry",
                desktop_file(&home).display().to_string(),
                desktop_file(&home).is_file(),
            ),
            ShellMenuItem::new(
                "linux",
                "Nautilus script",
                nautilus_script(&home).display().to_string(),
                nautilus_script(&home).is_file(),
            ),
        ]
    }

    pub(super) fn install(exe: &Path) -> Result<Vec<String>> {
        let home = home_dir()?;
        let desktop = desktop_file(&home);
        let script = nautilus_script(&home);
        atomic::write_atomic_str(&desktop, &desktop_entry(exe))?;
        atomic::write_atomic_str(&script, &nautilus_body(exe))?;
        set_executable(&script)?;
        debug!(desktop = %desktop.display(), script = %script.display(), "linux shell menu written");
        let mut actions = vec![
            format!("wrote {}", desktop.display()),
            format!("wrote {}", script.display()),
            format!("marked {} executable", script.display()),
        ];
        refresh_database(&home, &mut actions)?;
        info!(entries = actions.len(), "linux shell menu installed");
        Ok(actions)
    }

    pub(super) fn uninstall() -> Result<Vec<String>> {
        let home = home_dir()?;
        let mut actions = Vec::new();
        for path in [desktop_file(&home), nautilus_script(&home)] {
            if path.exists() {
                std::fs::remove_file(&path).at(&path)?;
                actions.push(format!("removed {}", path.display()));
            } else {
                actions.push(format!("{} was not present", path.display()));
            }
        }
        refresh_database(&home, &mut actions)?;
        info!("linux shell menu uninstalled");
        Ok(actions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_on_target_oses() {
        // This test runs on whichever platform CI/developers use; on the three
        // supported targets `supported()` must be true.
        if cfg!(windows) || cfg!(target_os = "macos") || cfg!(target_os = "linux") {
            assert!(supported());
        }
    }

    #[test]
    fn windows_entries_cover_all_keys() {
        let exe = Path::new("mlo.exe");
        let entries = windows::install_entries(exe);
        let keys: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();
        assert!(keys.contains(&windows::DIR_KEY));
        assert!(keys.contains(&windows::FILE_KEY));
        assert!(keys.contains(&windows::BG_KEY));
        assert!(keys.contains(&windows::APP_PATHS_KEY));
        assert!(keys.contains(&r"HKCU\Software\Classes\Directory\shell\mlo\command"));
        assert!(keys.contains(&r"HKCU\Software\Classes\*\shell\mlo\command"));
        assert!(keys.contains(&r"HKCU\Software\Classes\Directory\Background\shell\mlo\command"));
        assert_eq!(entries.len(), 7);
    }

    #[test]
    fn windows_commands_use_placeholders() {
        let entries = windows::install_entries(Path::new("mlo.exe"));
        let dir_cmd = entries
            .iter()
            .find(|e| e.key == r"HKCU\Software\Classes\Directory\shell\mlo\command")
            .unwrap();
        assert_eq!(dir_cmd.data, r#""mlo.exe" --open "%1""#);
        let bg_cmd = entries
            .iter()
            .find(|e| e.key == r"HKCU\Software\Classes\Directory\Background\shell\mlo\command")
            .unwrap();
        assert_eq!(bg_cmd.data, r#""mlo.exe" --open "%V""#);
        let label = entries.iter().find(|e| e.key == windows::DIR_KEY).unwrap();
        assert_eq!(label.data, "Open in mlo");
    }

    #[test]
    fn windows_add_argv_shape() {
        let entry = windows::RegEntry {
            key: windows::DIR_KEY.into(),
            name: None,
            data: "Open in mlo".into(),
        };
        assert_eq!(
            windows::add_argv(&entry),
            vec![
                "add".to_string(),
                windows::DIR_KEY.to_string(),
                "/ve".to_string(),
                "/d".to_string(),
                "Open in mlo".to_string(),
                "/f".to_string(),
            ]
        );
    }

    #[test]
    fn windows_delete_argv_and_keys() {
        assert_eq!(
            windows::delete_argv(windows::APP_PATHS_KEY),
            vec![
                "delete".to_string(),
                windows::APP_PATHS_KEY.to_string(),
                "/f".to_string(),
            ]
        );
        assert_eq!(windows::delete_keys().len(), 4);
    }

    #[test]
    fn macos_info_plist_is_a_plist() {
        let plist = macos::info_plist();
        assert!(plist.starts_with("<?xml"));
        assert!(plist.contains("<key>CFBundleIdentifier</key>"));
        assert!(plist.contains("<string>dev.mlo.shellmenu</string>"));
        assert!(plist.trim_end().ends_with("</plist>"));
    }

    #[test]
    fn macos_wflow_runs_exe_on_selection() {
        let exe = Path::new("/opt/mlo/mlo");
        let wflow = macos::document_wflow(exe);
        assert!(wflow.contains("<key>serviceInputTypeIdentifier</key>"));
        assert!(wflow.contains("<string>com.apple.Automator.fileSystemObject</string>"));
        assert!(wflow.contains("Run Shell Script"));
        // XML text needs no quote escaping, so the script keeps literal quotes.
        assert!(wflow.contains("--open \"$1\""));
        assert!(wflow.contains("/opt/mlo/mlo"));
        // The pure script builder is what lands in the plist.
        assert_eq!(
            macos::script(exe),
            "#!/bin/sh\n\"/opt/mlo/mlo\" --open \"$1\"\n"
        );
    }

    #[test]
    fn linux_desktop_entry_fields() {
        let exe = Path::new("mlo");
        let entry = linux::desktop_entry(exe);
        assert!(entry.contains("[Desktop Entry]\n"));
        assert!(entry.contains("Type=Application\n"));
        assert!(entry.contains("Name=mlo\n"));
        assert!(entry.contains("Exec=mlo --open %F\n"));
        assert!(entry.contains(
            "MimeType=inode/directory;audio/flac;audio/mpeg;audio/ogg;audio/mp4;audio/x-wav;video/x-matroska;\n"
        ));
        assert!(entry.contains("Terminal=true\n"));
    }

    #[test]
    fn linux_nautilus_body_uses_selection_env() {
        let body = linux::nautilus_body(Path::new("mlo"));
        assert!(body.starts_with("#!/bin/sh\n"));
        assert!(body.contains("--open \"$NAUTILUS_SCRIPT_SELECTED_FILE_PATHS\""));
    }

    #[test]
    fn paths_with_spaces_are_quoted() {
        let exe = Path::new("/opt/my mlo/mlo");

        let desktop = linux::desktop_entry(exe);
        assert!(desktop.contains("Exec=\"/opt/my mlo/mlo\" --open %F\n"));

        let nautilus = linux::nautilus_body(exe);
        assert!(nautilus.contains("exec '/opt/my mlo/mlo' --open"));

        let entries = windows::install_entries(exe);
        let dir_cmd = entries
            .iter()
            .find(|e| e.key == r"HKCU\Software\Classes\Directory\shell\mlo\command")
            .unwrap();
        assert!(dir_cmd.data.starts_with('"'));
        assert!(dir_cmd.data.contains("--open \"%1\""));
    }

    #[test]
    fn quoting_helpers_leave_simple_paths_alone() {
        assert_eq!(linux::desktop_quote("/usr/bin/mlo"), "/usr/bin/mlo");
        assert_eq!(linux::shell_quote("/usr/bin/mlo"), "/usr/bin/mlo");
        assert_eq!(linux::desktop_quote("a b"), "\"a b\"");
        assert_eq!(linux::shell_quote("it's"), "'it'\\''s'");
    }
}