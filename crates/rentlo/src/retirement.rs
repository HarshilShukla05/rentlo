use std::io::ErrorKind;
use std::path::Path;

const RETIRED_SESSIONS: [&str; 2] = ["whatsapp.db", "whatsapp-relink.db"];
const SIDECARS: [&str; 4] = ["", "-journal", "-shm", "-wal"];

/// Retire only known former pairing stores on the worker's data volume.
pub fn retire_sessions(active: &Path) -> Result<(), String> {
    let volume = Path::new("/data");
    if active.parent() != Some(volume) {
        return Err("WA_SESSION must be a fresh database under /data to retire old sessions".into());
    }
    remove_retired(volume, active)
}

fn remove_retired(volume: &Path, active: &Path) -> Result<(), String> {
    let retired =
        RETIRED_SESSIONS.iter().flat_map(|name| SIDECARS.map(|suffix| volume.join(format!("{name}{suffix}"))));
    if retired.clone().any(|file| file == active) {
        return Err("WA_SESSION points to a retired session; use a fresh database".into());
    }
    let mut removed = 0;
    for file in retired {
        match std::fs::remove_file(file) {
            Ok(()) => removed += 1,
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(format!("could not retire a former WhatsApp session file: {error}")),
        }
    }
    tracing::info!(removed, "retired former WhatsApp session files");
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn only_removes_retired_sessions_and_their_sidecars() {
        let root = std::env::temp_dir().join(format!("rentlo-retirement-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let active = root.join("current.db");
        let unrelated = root.join("other.db");
        let retired = root.join("whatsapp-relink.db");
        let journal = root.join("whatsapp.db-wal");
        for path in [&active, &unrelated, &retired, &journal] {
            std::fs::write(path, []).unwrap();
        }
        assert!(remove_retired(&root, &retired).is_err());
        assert!(journal.exists());
        remove_retired(&root, &active).unwrap();
        remove_retired(&root, &active).unwrap();
        assert!(!retired.exists() && !journal.exists());
        assert!(active.exists() && unrelated.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
