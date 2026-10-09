//! The checks on the Agent endpoint's directory and socket.
//!
//! The directory is where an agent's account is let in, and it is usually under a
//! place every user can write to (`/Users/Shared` is), so the server does not take
//! it on trust. It must be a real directory, not a link; owned by the account that
//! runs the server; and not writable by its group or by anyone else, or another user
//! could put something of their own where the socket is. A directory that is missing
//! is made at `0700`. After the socket is bound the same facts are read again, since
//! the checks and the bind are not one step.
//!
//! The decisions are functions of [`Facts`], which are plain values read from `lstat`
//! (which does not follow a link), so a directory owned by another uid, which a test
//! cannot make without privilege, is tested all the same.

use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt};
use std::path::Path;

/// The group-write and other-write bits.
const WRITABLE_BY_OTHERS: u32 = 0o022;

/// What `lstat` says about a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facts {
    pub is_directory: bool,
    pub is_socket: bool,
    pub is_link: bool,
    pub uid: u32,
    pub mode: u32,
    /// Which file this is, so a directory swapped for another between two looks is
    /// seen even when both pass.
    pub device: u64,
    pub inode: u64,
}

impl Facts {
    pub fn of(path: &Path) -> std::io::Result<Self> {
        let metadata = std::fs::symlink_metadata(path)?;
        let kind = metadata.file_type();
        Ok(Self {
            is_directory: kind.is_dir(),
            is_socket: kind.is_socket(),
            is_link: kind.is_symlink(),
            uid: metadata.uid(),
            mode: metadata.mode() & 0o7777,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}

/// Why `facts` is not a directory the Agent endpoint may use, if it is not.
pub fn directory_problem(facts: &Facts, server_uid: u32) -> Option<String> {
    if facts.is_link {
        return Some("is a link, not a directory".into());
    }
    if !facts.is_directory {
        return Some("is not a directory".into());
    }
    if facts.uid != server_uid {
        return Some("is not owned by the account that runs the server".into());
    }
    if facts.mode & WRITABLE_BY_OTHERS != 0 {
        return Some("is writable by its group or by everyone".into());
    }
    None
}

/// The directory ready for the socket: checked, or made at `0700` when it is
/// missing. The error is the reason the Agent endpoint is not created.
pub fn prepare(directory: &Path, server_uid: u32) -> Result<Facts, String> {
    let reason = |why: String| {
        format!(
            "the Agent endpoint's directory {} {why}",
            directory.display()
        )
    };
    if !directory.is_absolute() {
        return Err(reason("is not an absolute path".into()));
    }
    // `lstat` follows a link that is not the last component, and a trailing `/`, `.`
    // or `..` makes the last one not last, which would hide a link from the check.
    let dotted = directory.components().any(|part| {
        matches!(
            part,
            std::path::Component::CurDir | std::path::Component::ParentDir
        )
    });
    let bytes = directory.as_os_str().as_bytes();
    // `components` drops a trailing `.`, so the bytes are looked at as well.
    let trailing = bytes.len() > 1 && (bytes.ends_with(b"/") || bytes.ends_with(b"/."));
    if trailing || dotted {
        return Err(reason(
            "must be written without a trailing `/`, `.` or `..`".into(),
        ));
    }
    let facts = match Facts::of(directory) {
        Ok(facts) => facts,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(directory)
                .map_err(|error| reason(format!("cannot be made: {error}")))?;
            Facts::of(directory).map_err(|error| reason(format!("cannot be read: {error}")))?
        }
        Err(error) => return Err(reason(format!("cannot be read: {error}"))),
    };
    match directory_problem(&facts, server_uid) {
        Some(problem) => Err(reason(problem)),
        None => Ok(facts),
    }
}

/// Why the socket just bound, or the directory around it, is not what was checked.
/// `before` is the directory as `prepare` found it.
pub fn after_bind_problem(
    before: &Facts,
    directory: &Facts,
    socket: &Facts,
    server_uid: u32,
) -> Option<String> {
    if (directory.device, directory.inode) != (before.device, before.inode) {
        return Some("the directory was replaced while the socket was being made".into());
    }
    if let Some(problem) = directory_problem(directory, server_uid) {
        return Some(format!("the directory changed and now {problem}"));
    }
    if !socket.is_socket || socket.uid != server_uid {
        return Some("what is at the socket's path is not the socket this server made".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVER: u32 = 501;

    fn directory() -> Facts {
        Facts {
            is_directory: true,
            is_socket: false,
            is_link: false,
            uid: SERVER,
            mode: 0o700,
            device: 1,
            inode: 10,
        }
    }

    fn socket() -> Facts {
        Facts {
            is_directory: false,
            is_socket: true,
            mode: 0o600,
            inode: 11,
            ..directory()
        }
    }

    #[test]
    fn a_path_that_hides_a_link_from_lstat_is_refused() {
        let base = std::env::temp_dir().join(format!("agent-dir-{}", std::process::id()));
        let real = base.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = base.join("link");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let uid = Facts::of(&real).unwrap().uid;
        for path in [
            format!("{}/", link.display()),
            format!("{}/.", link.display()),
            format!("{}/../real", link.display()),
        ] {
            let refused = prepare(Path::new(&path), uid).unwrap_err();
            assert!(refused.contains("without a trailing"), "{path}: {refused}");
        }
        assert!(prepare(&link, uid).unwrap_err().contains("is a link"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_private_directory_of_the_servers_own_is_accepted() {
        assert_eq!(directory_problem(&directory(), SERVER), None);
        // Readable by others is fine; it is writing that lets them plant a socket.
        let readable = Facts {
            mode: 0o755,
            ..directory()
        };
        assert_eq!(directory_problem(&readable, SERVER), None);
    }

    #[test]
    fn a_directory_owned_by_someone_else_is_refused_even_if_it_is_private() {
        let theirs = Facts {
            uid: 0,
            ..directory()
        };
        assert!(directory_problem(&theirs, SERVER)
            .unwrap()
            .contains("not owned"));
    }

    #[test]
    fn a_link_a_file_and_a_directory_others_can_write_are_each_refused() {
        let link = Facts {
            is_link: true,
            is_directory: false,
            ..directory()
        };
        assert!(directory_problem(&link, SERVER).unwrap().contains("link"));
        let file = Facts {
            is_directory: false,
            ..directory()
        };
        assert!(directory_problem(&file, SERVER)
            .unwrap()
            .contains("not a directory"));
        for mode in [0o770, 0o707, 0o777, 0o720, 0o702, 0o1777] {
            let wide = Facts {
                mode,
                ..directory()
            };
            assert!(
                directory_problem(&wide, SERVER)
                    .unwrap()
                    .contains("writable"),
                "{mode:o}"
            );
        }
    }

    #[test]
    fn a_socket_that_is_not_ours_after_the_bind_is_refused() {
        let ours = after_bind_problem(&directory(), &directory(), &socket(), SERVER);
        assert_eq!(ours, None);

        // Someone else's socket is at the path.
        let theirs = Facts { uid: 0, ..socket() };
        assert!(
            after_bind_problem(&directory(), &directory(), &theirs, SERVER)
                .unwrap()
                .contains("not the socket this server made")
        );

        // Something that is not a socket at all.
        let file = Facts {
            is_socket: false,
            ..socket()
        };
        assert!(after_bind_problem(&directory(), &directory(), &file, SERVER).is_some());
    }

    #[test]
    fn a_directory_swapped_or_loosened_after_the_check_is_refused() {
        let swapped = Facts {
            inode: 99,
            ..directory()
        };
        assert!(
            after_bind_problem(&directory(), &swapped, &socket(), SERVER)
                .unwrap()
                .contains("replaced")
        );

        let loosened = Facts {
            mode: 0o777,
            ..directory()
        };
        assert!(
            after_bind_problem(&directory(), &loosened, &socket(), SERVER)
                .unwrap()
                .contains("changed")
        );
    }
}
