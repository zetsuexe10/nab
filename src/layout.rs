use std::path::{Path, PathBuf};

pub const APP: &str = "Nab";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Folder {
    Videos,
    Audios,
    Galleries,
}

impl Folder {
    pub fn name(self) -> &'static str {
        match self {
            Folder::Videos => "Videos",
            Folder::Audios => "Audios",
            Folder::Galleries => "Galleries",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    pub fn new(root: PathBuf) -> Layout {
        Layout { root }
    }

    pub fn dir(&self, site: &str, folder: Folder) -> PathBuf {
        self.root.join(site).join(folder.name())
    }
}

pub fn default_root() -> PathBuf {
    downloads().join(APP)
}

pub struct Host<'a> {
    pub windows: bool,
    pub env: &'a dyn Fn(&str) -> Option<String>,
    pub read: &'a dyn Fn(&Path) -> Option<String>,
}

pub fn downloads() -> PathBuf {
    let env = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    let read = |path: &Path| std::fs::read_to_string(path).ok();
    downloads_on(&Host {
        windows: cfg!(windows),
        env: &env,
        read: &read,
    })
}

pub fn downloads_on(host: &Host) -> PathBuf {
    let home = if host.windows {
        (host.env)("USERPROFILE")
            .or_else(|| match ((host.env)("HOMEDRIVE"), (host.env)("HOMEPATH")) {
                (Some(drive), Some(path)) => Some(format!("{drive}{path}")),
                _ => None,
            })
            .or_else(|| (host.env)("HOME"))
    } else {
        (host.env)("HOME")
    };
    let Some(home) = home.map(PathBuf::from) else {
        return PathBuf::from("Downloads");
    };
    if !host.windows {
        if let Some(dir) = (host.env)("XDG_DOWNLOAD_DIR") {
            return PathBuf::from(dir);
        }
        let config = (host.env)("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        if let Some(found) = (host.read)(&config.join("user-dirs.dirs"))
            .and_then(|text| user_dir(&text, "XDG_DOWNLOAD_DIR", &home))
        {
            return found;
        }
    }
    home.join("Downloads")
}

fn user_dir(text: &str, key: &str, home: &Path) -> Option<PathBuf> {
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line
            .strip_prefix(key)
            .and_then(|rest| rest.strip_prefix('='))
        else {
            continue;
        };
        let value = rest.trim().trim_matches('"');
        if value.is_empty() {
            return None;
        }
        let resolved = match value.strip_prefix("$HOME") {
            Some(tail) => format!("{}{tail}", home.display()),
            None => value.to_string(),
        };
        let path = PathBuf::from(resolved);
        return (path != home).then_some(path);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn linux_follows_xdg_then_user_dirs_then_home() {
        let none = |_: &Path| None::<String>;
        let set = env(&[("HOME", "/home/zed"), ("XDG_DOWNLOAD_DIR", "/data/dl")]);
        let host = Host {
            windows: false,
            env: &set,
            read: &none,
        };
        assert_eq!(downloads_on(&host), PathBuf::from("/data/dl"));

        let plain = env(&[("HOME", "/home/zed")]);
        let dirs = |path: &Path| {
            assert_eq!(path, Path::new("/home/zed/.config/user-dirs.dirs"));
            Some(
                "XDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_DOWNLOAD_DIR=\"$HOME/Grabbed\"\n"
                    .to_string(),
            )
        };
        let host = Host {
            windows: false,
            env: &plain,
            read: &dirs,
        };
        assert_eq!(downloads_on(&host), PathBuf::from("/home/zed/Grabbed"));

        let host = Host {
            windows: false,
            env: &plain,
            read: &none,
        };
        assert_eq!(
            downloads_on(&host),
            Path::new("/home/zed").join("Downloads")
        );

        let custom = env(&[("HOME", "/home/zed"), ("XDG_CONFIG_HOME", "/cfg")]);
        let seen = |path: &Path| {
            assert_eq!(path, Path::new("/cfg/user-dirs.dirs"));
            Some("XDG_DOWNLOAD_DIR=\"/mnt/files\"".to_string())
        };
        let host = Host {
            windows: false,
            env: &custom,
            read: &seen,
        };
        assert_eq!(downloads_on(&host), PathBuf::from("/mnt/files"));

        let homeless = Host {
            windows: false,
            env: &env(&[]),
            read: &none,
        };
        assert_eq!(downloads_on(&homeless), PathBuf::from("Downloads"));
    }

    #[test]
    fn a_user_dirs_entry_pointing_at_home_is_ignored() {
        let none = |_: &Path| None::<String>;
        let plain = env(&[("HOME", "/home/zed")]);
        let dirs = |_: &Path| Some("XDG_DOWNLOAD_DIR=\"$HOME\"".to_string());
        let host = Host {
            windows: false,
            env: &plain,
            read: &dirs,
        };
        assert_eq!(
            downloads_on(&host),
            Path::new("/home/zed").join("Downloads")
        );
        let _ = none;
    }

    #[test]
    fn windows_uses_the_profile_folder() {
        let none = |_: &Path| None::<String>;
        let profile = env(&[("USERPROFILE", "C:\\Users\\Zed"), ("HOME", "/ignored")]);
        let host = Host {
            windows: true,
            env: &profile,
            read: &none,
        };
        assert_eq!(
            downloads_on(&host),
            Path::new("C:\\Users\\Zed").join("Downloads")
        );

        let split = env(&[("HOMEDRIVE", "D:"), ("HOMEPATH", "\\Users\\Zed")]);
        let host = Host {
            windows: true,
            env: &split,
            read: &none,
        };
        assert_eq!(
            downloads_on(&host),
            Path::new("D:\\Users\\Zed").join("Downloads")
        );

        let xdg = env(&[
            ("USERPROFILE", "C:\\Users\\Zed"),
            ("XDG_DOWNLOAD_DIR", "/nope"),
        ]);
        let host = Host {
            windows: true,
            env: &xdg,
            read: &none,
        };
        assert_eq!(
            downloads_on(&host),
            Path::new("C:\\Users\\Zed").join("Downloads")
        );
    }

    #[test]
    fn folders_nest_site_then_kind() {
        let layout = Layout::new(PathBuf::from("/dl").join(APP));
        assert_eq!(
            layout.dir("Youtube", Folder::Videos),
            Path::new("/dl").join("Nab").join("Youtube").join("Videos")
        );
        assert_eq!(
            layout.dir("Xhamster", Folder::Galleries),
            Path::new("/dl")
                .join("Nab")
                .join("Xhamster")
                .join("Galleries")
        );
    }
}
