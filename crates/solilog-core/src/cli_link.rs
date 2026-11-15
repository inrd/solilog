//! Putting the `solilog` command on the PATH from the macOS app: a symbolic
//! link (`/usr/local/bin/solilog`) to the binary inside the app bundle, as
//! Zed and VS Code do. The app runs this (`workspace::InstallCommandLineTool`);
//! on Linux the install step of the download does it instead.
//!
//! Like `skill.rs`, this is installation support, not a domain operation: it
//! lives in core so it can be tested without gpui, against temporary folders.
//! The app writes the link itself when it can, and otherwise runs
//! [`admin_script`] with administrator rights.
//!
//! A link is only ever replaced by another link: a real file at its place
//! (a binary copied there by hand, a package manager's) is left alone.

use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

/// Where the app links the command: on the PATH by default on macOS.
pub const CLI_LINK_PATH: &str = "/usr/local/bin/solilog";

/// What is at the link's place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CliLinkState {
    /// Nothing (or a link to nothing, which is the same: it gets replaced).
    Missing,
    /// A link to this binary already.
    Current,
    /// A link to another file: an older copy of the app, or a build from
    /// source (`make install`).
    Elsewhere(PathBuf),
    /// Something that isn't a link; never replaced.
    NotALink,
}

/// What is at `link`, compared with `target` (the binary it should point to).
pub fn cli_link_state(link: &Path, target: &Path) -> Result<CliLinkState> {
    let meta = match fs::symlink_metadata(link) {
        Ok(meta) => meta,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(CliLinkState::Missing),
        Err(err) => return Err(err).with_context(|| format!("reading {}", link.display())),
    };
    if !meta.file_type().is_symlink() {
        return Ok(CliLinkState::NotALink);
    }
    let points_to =
        fs::read_link(link).with_context(|| format!("reading the link {}", link.display()))?;
    let points_to = link.parent().unwrap_or(Path::new("/")).join(points_to);
    let Ok(resolved) = points_to.canonicalize() else {
        // Dangling: its file is gone (an app moved or deleted).
        return Ok(CliLinkState::Missing);
    };
    let target = target
        .canonicalize()
        .unwrap_or_else(|_| target.to_path_buf());
    Ok(if resolved == target {
        CliLinkState::Current
    } else {
        CliLinkState::Elsewhere(points_to)
    })
}

/// The binary to link to, from the running one (`std::env::current_exe`):
/// it must be inside an app bundle that stays where it is.
pub fn app_bundle_binary(exe: &Path) -> Result<PathBuf> {
    let exe = exe.canonicalize().unwrap_or_else(|_| exe.to_path_buf());
    let text = exe.to_string_lossy();
    anyhow::ensure!(
        text.contains(".app/Contents/MacOS/"),
        "Solilog isn't running from its app ({}): install Solilog.app and run this from it",
        exe.display()
    );
    // macOS runs a downloaded app from a temporary read-only copy until it
    // is moved (App Translocation); a link there would break.
    anyhow::ensure!(
        !text.contains("/AppTranslocation/"),
        "Solilog is running from a temporary copy: move Solilog.app to Applications, open it from there and try again"
    );
    Ok(exe)
}

/// Makes `link` a link to `target`, without administrator rights. Fails
/// with [`ErrorKind::PermissionDenied`] when they are needed (the folder
/// isn't writable or doesn't exist yet and can't be created): then run
/// [`admin_script`]. The caller has checked [`cli_link_state`]; this checks
/// again that it replaces a link only.
pub fn install_cli_link(link: &Path, target: &Path) -> io::Result<()> {
    let dir = link.parent().unwrap_or(Path::new("/"));
    fs::create_dir_all(dir)?;
    if let Ok(meta) = fs::symlink_metadata(link)
        && !meta.file_type().is_symlink()
    {
        return Err(io::Error::new(
            ErrorKind::AlreadyExists,
            format!("{} isn't a link; not replacing it", link.display()),
        ));
    }
    // Made aside, then renamed over the old link: one atomic step.
    let name = link.file_name().unwrap_or_default().to_string_lossy();
    let temp = dir.join(format!(".{name}.{}", std::process::id()));
    let _ = fs::remove_file(&temp);
    symlink(target, &temp)?;
    fs::rename(&temp, link).inspect_err(|_| {
        let _ = fs::remove_file(&temp);
    })
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn symlink(_: &Path, _: &Path) -> io::Result<()> {
    Err(io::Error::new(
        ErrorKind::Unsupported,
        "links to the command are only made on macOS",
    ))
}

/// A shell script doing what [`install_cli_link`] does, to run with
/// administrator rights (`do shell script … with administrator privileges`).
/// It exits with [`NOT_A_LINK_EXIT`] rather than replace something that
/// isn't a link.
pub fn admin_script(link: &Path, target: &Path) -> String {
    let dir = link.parent().unwrap_or(Path::new("/"));
    let (l, d, t) = (
        shell_quote(&link.to_string_lossy()),
        shell_quote(&dir.to_string_lossy()),
        shell_quote(&target.to_string_lossy()),
    );
    format!(
        "if [ -e {l} ] && [ ! -L {l} ]; then exit {NOT_A_LINK_EXIT}; fi; \
         mkdir -p {d} && ln -sfn {t} {l}"
    )
}

/// The exit code of [`admin_script`] when the link's place holds a file.
pub const NOT_A_LINK_EXIT: i32 = 3;

/// `s` as one shell word.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// `s` as an AppleScript string literal.
pub fn applescript_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', r"\\").replace('"', "\\\""))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("solilog-cli-link-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir.canonicalize().unwrap())
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn binary(dir: &Path, app: &str) -> PathBuf {
        let path = dir.join(format!("{app}/Contents/MacOS/solilog"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "binary").unwrap();
        path
    }

    #[test]
    fn links_then_recognizes_the_link() {
        let tmp = TempDir::new("links");
        let target = binary(&tmp.0, "Solilog.app");
        let link = tmp.0.join("bin/solilog");
        assert_eq!(
            cli_link_state(&link, &target).unwrap(),
            CliLinkState::Missing
        );
        install_cli_link(&link, &target).unwrap();
        assert_eq!(
            cli_link_state(&link, &target).unwrap(),
            CliLinkState::Current
        );
        // Again: nothing changes.
        install_cli_link(&link, &target).unwrap();
        assert_eq!(fs::read_link(&link).unwrap(), target);
    }

    #[test]
    fn replaces_a_link_to_elsewhere() {
        let tmp = TempDir::new("elsewhere");
        let old = binary(&tmp.0, "Old/Solilog.app");
        let target = binary(&tmp.0, "Solilog.app");
        let link = tmp.0.join("solilog");
        std::os::unix::fs::symlink(&old, &link).unwrap();
        assert_eq!(
            cli_link_state(&link, &target).unwrap(),
            CliLinkState::Elsewhere(old)
        );
        install_cli_link(&link, &target).unwrap();
        assert_eq!(
            cli_link_state(&link, &target).unwrap(),
            CliLinkState::Current
        );
    }

    #[test]
    fn a_dangling_link_counts_as_missing() {
        let tmp = TempDir::new("dangling");
        let target = binary(&tmp.0, "Solilog.app");
        let link = tmp.0.join("solilog");
        std::os::unix::fs::symlink(tmp.0.join("Gone.app/Contents/MacOS/solilog"), &link).unwrap();
        assert_eq!(
            cli_link_state(&link, &target).unwrap(),
            CliLinkState::Missing
        );
        install_cli_link(&link, &target).unwrap();
        assert_eq!(
            cli_link_state(&link, &target).unwrap(),
            CliLinkState::Current
        );
    }

    #[test]
    fn never_replaces_a_real_file() {
        let tmp = TempDir::new("real");
        let target = binary(&tmp.0, "Solilog.app");
        let link = tmp.0.join("solilog");
        fs::write(&link, "someone else's").unwrap();
        assert_eq!(
            cli_link_state(&link, &target).unwrap(),
            CliLinkState::NotALink
        );
        let err = install_cli_link(&link, &target).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&link).unwrap(), "someone else's");
    }

    #[test]
    fn admin_script_links_and_refuses_real_files() {
        let tmp = TempDir::new("script");
        let target = binary(&tmp.0, "It's here.app");
        let link = tmp.0.join("new dir/solilog");
        let run = |script: String| {
            std::process::Command::new("sh")
                .arg("-c")
                .arg(script)
                .status()
                .unwrap()
                .code()
        };
        assert_eq!(run(admin_script(&link, &target)), Some(0));
        assert_eq!(
            cli_link_state(&link, &target).unwrap(),
            CliLinkState::Current
        );
        fs::remove_file(&link).unwrap();
        fs::write(&link, "file").unwrap();
        assert_eq!(run(admin_script(&link, &target)), Some(NOT_A_LINK_EXIT));
        assert_eq!(fs::read_to_string(&link).unwrap(), "file");
    }

    #[test]
    fn requires_an_app_bundle() {
        let tmp = TempDir::new("bundle");
        let inside = binary(&tmp.0, "Solilog.app");
        assert_eq!(app_bundle_binary(&inside).unwrap(), inside);
        assert!(app_bundle_binary(Path::new("/src/solilog/target/debug/solilog")).is_err());
        assert!(
            app_bundle_binary(Path::new(
                "/private/var/folders/x/AppTranslocation/1234/d/Solilog.app/Contents/MacOS/solilog"
            ))
            .is_err()
        );
    }

    #[test]
    fn quotes_for_applescript() {
        assert_eq!(applescript_string(r#"a "b" \c"#), r#""a \"b\" \\c""#);
    }
}
