// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! La parte automatica di alcuni test manuali del livello 4 del piano di test
//! (`docs/piano-di-test.md`): ciò che si verifica da programma, senza interazione, con gli
//! strumenti del sistema come oracolo indipendente dal codice dell'app.
//!
//! - M-22 (Windows): le chiavi del clic destro scritte in una radice di prova di
//!   `HKCU\Software`, lette con `reg.exe`, e tolte allo spegnimento;
//! - M-24 (Linux): lo script di Nautilus ha i permessi `-rwx------` e il testo atteso;
//! - M-26 (Windows): un file compresso con WOF (`compact /c /exe:lzx`) si legge.
//!
//! Il resto di quei casi (Explorer, Nautilus, file solo online) resta manuale. Sono test
//! `#[ignore]`, lanciati da `cargo xtask test-plan`: toccano il registro e gli strumenti del
//! sistema, che i test normali non usano.

#[cfg(windows)]
mod windows {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use lynkeus_app::context_menu::{ContextMenuStore, MenuEntry, MenuTarget};
    use lynkeus_app::form::{RootAdded, RootKind};
    use lynkeus_app::platform::ContextMenu;
    use lynkeus_app::{Application, Pump, Settings};
    use lynkeus_engine::Notify;
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};

    /// Tempo massimo di uno strumento del sistema o di una ricerca.
    const DEADLINE: Duration = Duration::from_secs(60);

    /// Un programma di `%SystemRoot%\System32`, per percorso assoluto.
    fn system32(program: &str) -> PathBuf {
        let root = std::env::var_os("SystemRoot").expect("variabile SystemRoot");
        Path::new(&root).join("System32").join(program)
    }

    /// Una radice di prova propria in `HKCU\Software`, tolta alla fine (anche se il test
    /// fallisce).
    struct TestRoot {
        parent: RegKey,
        name: String,
    }

    impl TestRoot {
        fn new() -> TestRoot {
            let mut bytes = [0u8; 8];
            getrandom::fill(&mut bytes).expect("byte casuali");
            let suffix: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            let parent = RegKey::predef(HKEY_CURRENT_USER)
                .open_subkey_with_flags("Software", KEY_READ | KEY_WRITE)
                .expect("HKCU\\Software");
            let name = format!("LynkeusSearcherPlanTest-{suffix}");
            parent.create_subkey(&name).expect("radice di prova");
            TestRoot { parent, name }
        }

        /// Il percorso completo per `reg.exe` di `relative` sotto la radice.
        fn reg_path(&self, relative: &str) -> String {
            format!(r"HKCU\Software\{}\{relative}", self.name)
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            // Già tolta o non tolta: nel secondo caso resta una chiave di prova innocua.
            if self.parent.delete_subkey_all(&self.name).is_err() {}
        }
    }

    /// `reg.exe query <key> [/ve]`: l'uscita se la chiave (o il suo valore predefinito)
    /// esiste, `None` altrimenti.
    fn reg_query(key: &str, default_value: bool) -> Option<String> {
        let mut command = Command::new(system32("reg.exe"));
        command.args(["query", key]);
        if default_value {
            command.arg("/ve");
        }
        let output = command
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .expect("avvio di reg.exe");
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Il testo del valore predefinito `REG_SZ` nell'uscita di `reg query /ve`.
    fn default_string(query_output: &str) -> Option<String> {
        query_output.lines().find_map(|line| {
            let (_, value) = line.split_once("REG_SZ")?;
            Some(value.trim_start().to_owned())
        })
    }

    /// M-22: voce scritta nelle tre chiavi con la riga di comando attesa (lettura con
    /// `reg.exe`), poi tolta del tutto.
    #[test]
    #[ignore = "M-22 (parte automatica), lanciato da cargo xtask test-plan"]
    fn m22_context_menu_keys_in_a_test_root() {
        let root = TestRoot::new();
        let store = ContextMenu::with_classes_path(&format!(r"Software\{}", root.name));
        let exe = Path::new(r"C:\Programmi\Lynkeus Searcher\lynkeus-searcher.exe");
        let entry = MenuEntry::new(exe, "Cerca con Lynkeus Searcher", &[]).expect("voce valida");
        store.install(&entry).expect("voce scritta");
        for target in MenuTarget::ALL {
            let key = root.reg_path(&format!(r"{}\command", target.key_path()));
            let output = reg_query(&key, true).unwrap_or_else(|| panic!("chiave assente: {key}"));
            assert_eq!(
                default_string(&output).as_deref(),
                Some(entry.windows_command(target).as_str()),
                "{key}: {output}"
            );
        }
        let background = root.reg_path(&format!(
            r"{}\command",
            MenuTarget::FolderBackground.key_path()
        ));
        let line = reg_query(&background, true).and_then(|o| default_string(&o));
        assert_eq!(
            line.as_deref(),
            Some(r#""C:\Programmi\Lynkeus Searcher\lynkeus-searcher.exe" --add "%V\.""#)
        );

        store.remove(&entry).expect("voce tolta");
        for target in MenuTarget::ALL {
            let key = root.reg_path(&target.key_path());
            assert!(reg_query(&key, false).is_none(), "chiave rimasta: {key}");
        }
    }

    /// M-26: un file di testo compresso con WOF (LZX) si legge come gli altri.
    #[test]
    #[ignore = "M-26 (parte automatica), lanciato da cargo xtask test-plan"]
    fn m26_wof_compressed_files_are_read() {
        const LINES: usize = 2_000;
        let dir = tempfile::tempdir().expect("cartella temporanea");
        let file = dir.path().join("compresso.txt");
        std::fs::write(&file, "comprare la carta\n".repeat(LINES)).expect("file scritto");
        let status = Command::new(system32("compact.exe"))
            .args(["/c", "/exe:lzx"])
            .arg(&file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("avvio di compact.exe");
        assert!(status.success(), "compact /c /exe:lzx: {status}");

        let mut app = Application::build(Settings::default()).expect("app");
        assert_eq!(
            app.form.add_root(dir.path().to_owned(), RootKind::Folder),
            RootAdded::Added
        );
        app.set_term("ar");
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let notify: Notify = Arc::new(move || {
            // La ricerca può essere già letta: un avviso perso non conta.
            if tx.send(()).is_err() {}
        });
        app.start_search(notify).expect("ricerca avviata");
        let deadline = Instant::now() + DEADLINE;
        while app.pump_events(usize::MAX) != Pump::Ended {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "ricerca non finita entro {DEADLINE:?}");
            if rx.recv_timeout(left).is_err() {}
        }
        assert!(
            app.results().warnings.is_empty(),
            "{:?}",
            app.results().warnings
        );
        assert_eq!(app.results().hits.len(), LINES * 2, "comprare e carta");
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use lynkeus_app::context_menu::{ContextMenuStore, MenuEntry};
    use lynkeus_app::platform::ContextMenu;

    /// M-24: lo script di Nautilus è privato (`-rwx------`) e ha il testo atteso; spegnere
    /// la voce lo toglie.
    #[test]
    #[ignore = "M-24 (parte automatica), lanciato da cargo xtask test-plan"]
    fn m24_nautilus_script_is_private() {
        let dir = tempfile::tempdir().expect("cartella temporanea");
        let scripts = dir.path().join("scripts");
        let store = ContextMenu::with_scripts_dir(scripts.clone());
        let exe = Path::new("/opt/lynkeus-searcher/lynkeus-searcher");
        let entry = MenuEntry::new(exe, "Cerca con Lynkeus Searcher", &[]).expect("voce valida");
        store.install(&entry).expect("script scritto");
        let files: Vec<_> = std::fs::read_dir(&scripts)
            .expect("cartella degli script")
            .map(|item| item.expect("voce della cartella").path())
            .collect();
        assert_eq!(files.len(), 1, "{files:?}");
        let mode = std::fs::metadata(&files[0])
            .expect("metadati dello script")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700, "permessi {:o}", mode & 0o777);
        assert_eq!(
            std::fs::read_to_string(&files[0]).expect("script leggibile"),
            entry.nautilus_script()
        );
        store.remove(&entry).expect("script tolto");
        assert_eq!(
            std::fs::read_dir(&scripts)
                .expect("cartella degli script")
                .count(),
            0
        );
    }
}
