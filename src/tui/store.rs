use std::fs;
use std::path::PathBuf;

use super::theme::ThemeMode;
use crate::sites::Site;

const LIMIT: usize = 50;

#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn at(dir: PathBuf) -> Store {
        Store { dir }
    }

    pub fn locate() -> Option<Store> {
        if let Some(dir) = std::env::var_os("NAB_CONFIG_DIR").filter(|dir| !dir.is_empty()) {
            return Some(Store::at(PathBuf::from(dir)));
        }
        let base = if cfg!(windows) {
            std::env::var_os("APPDATA").map(PathBuf::from)
        } else {
            std::env::var_os("XDG_CONFIG_HOME")
                .filter(|dir| !dir.is_empty())
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
                })
        }?;
        Some(Store::at(base.join("nab")))
    }

    pub fn history(&self) -> Vec<String> {
        let Ok(text) = fs::read_to_string(self.dir.join("history.json")) else {
            return Vec::new();
        };
        serde_json::from_str::<Vec<String>>(&text)
            .map(|mut entries| {
                entries.retain(|entry| !entry.trim().is_empty());
                entries.truncate(LIMIT);
                entries
            })
            .unwrap_or_default()
    }

    pub fn remember(&self, history: &mut Vec<String>, entry: &str) {
        let entry = entry.trim();
        if entry.is_empty() {
            return;
        }
        history.retain(|existing| existing != entry);
        history.insert(0, entry.to_string());
        history.truncate(LIMIT);
        self.write(
            "history.json",
            &serde_json::to_string(history).unwrap_or_default(),
        );
    }

    pub fn theme(&self) -> Option<ThemeMode> {
        fs::read_to_string(self.dir.join("theme"))
            .ok()
            .and_then(|text| ThemeMode::parse(&text))
    }

    pub fn save_theme(&self, mode: ThemeMode) {
        self.write("theme", mode.label());
    }

    pub fn site(&self) -> Option<Site> {
        fs::read_to_string(self.dir.join("site"))
            .ok()
            .and_then(|text| Site::parse(&text))
    }

    pub fn save_site(&self, site: Site) {
        self.write("site", site.label());
    }

    fn write(&self, name: &str, text: &str) {
        if fs::create_dir_all(&self.dir).is_ok() {
            let _ = fs::write(self.dir.join(name), text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nab-store-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn history_keeps_newest_first_without_duplicates() {
        let dir = scratch("history");
        let store = Store::at(dir.clone());
        let mut history = store.history();
        assert!(history.is_empty());
        store.remember(&mut history, "one");
        store.remember(&mut history, "two");
        store.remember(&mut history, "  one  ");
        store.remember(&mut history, "   ");
        assert_eq!(history, vec!["one", "two"]);
        assert_eq!(Store::at(dir.clone()).history(), vec!["one", "two"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn history_is_capped() {
        let dir = scratch("cap");
        let store = Store::at(dir.clone());
        let mut history = Vec::new();
        for index in 0..80 {
            store.remember(&mut history, &format!("entry {index}"));
        }
        assert_eq!(history.len(), LIMIT);
        assert_eq!(history[0], "entry 79");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn theme_round_trips_and_garbage_is_ignored() {
        let dir = scratch("theme");
        let store = Store::at(dir.clone());
        assert_eq!(store.theme(), None);
        store.save_theme(ThemeMode::Dark);
        assert_eq!(store.theme(), Some(ThemeMode::Dark));
        fs::write(dir.join("theme"), "purple").unwrap();
        assert_eq!(store.theme(), None);
        assert_eq!(store.site(), None);
        store.save_site(Site::named("xhamster").unwrap());
        assert_eq!(store.site(), Some(Site::named("xhamster").unwrap()));
        fs::write(dir.join("history.json"), "{broken").unwrap();
        assert!(store.history().is_empty());
        let _ = fs::remove_dir_all(dir);
    }
}
