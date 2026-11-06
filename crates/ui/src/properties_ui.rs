use crate::{
    model::{Action, Model},
    properties_dialog::Dialog,
    properties_job::Reply,
    rename_job::Request,
};
use fltk::{
    app,
    enums::{Event, Key, Shortcut},
};
use std::path::PathBuf;

const MAX_OBJECTS: usize = 32;
const MAX_OBJECT_BYTES: usize = 64 * 1024;

fn bounded_objects(paths: Vec<PathBuf>) -> Result<Vec<PathBuf>, String> {
    if paths.is_empty() || paths.len() > MAX_OBJECTS {
        return Err("Propiedades: elige entre 1 y 32 objetos".into());
    }
    let mut bytes = 0usize;
    let mut objects = Vec::with_capacity(paths.len());
    for path in paths {
        files_core::navigation_history::validate(&path).map_err(str::to_owned)?;
        bytes = bytes
            .checked_add(path.as_os_str().len())
            .filter(|&n| n <= MAX_OBJECT_BYTES)
            .ok_or("Propiedades: las rutas superan el límite de 64 KiB")?;
        if !objects.contains(&path) {
            objects.push(path);
        }
    }
    Ok(objects)
}

impl Model {
    pub fn open_properties(&mut self) {
        if self.mutations.busy || self.mutation_applied || self.copy_busy() {
            return;
        }
        let indices: Vec<_> = if self.selection.is_empty() {
            self.selected.into_iter().collect()
        } else {
            self.order
                .iter()
                .copied()
                .filter(|i| self.selection.contains(i))
                .collect()
        };
        if indices.is_empty() {
            return;
        }
        if indices.len() > MAX_OBJECTS {
            self.operation_message = Some("Propiedades: elige entre 1 y 32 objetos".into());
            return;
        }
        let paths: Option<Vec<_>> = indices
            .iter()
            .map(|i| {
                self.entries
                    .get(*i)
                    .map(|entry| self.path.join(&entry.name))
            })
            .collect();
        let Some(paths) = paths else { return };
        if let Err(error) =
            bounded_objects(paths).and_then(|objects| self.start_properties(objects, 0))
        {
            self.operation_message = Some(error);
        }
    }
    /// Open explicit bounded paths without navigating or depending on browser
    /// selection. The existing inspection worker performs all filesystem I/O.
    pub fn open_properties_paths(&mut self, paths: Vec<PathBuf>) -> Result<(), String> {
        let objects = bounded_objects(paths)?;
        if crate::tab_gestures::blocked(self) || self.mutation_applied || self.copy_busy() {
            return Err("Propiedades: termina la edición, diálogo u operación actual".into());
        }
        self.start_properties(objects, 0)
    }
    fn start_properties(&mut self, objects: Vec<PathBuf>, index: usize) -> Result<(), String> {
        if self.mutations.busy || self.mutation_applied || self.copy_busy() {
            return Err("Propiedades: espera a que termine la operación actual".into());
        }
        let path = objects
            .get(index)
            .ok_or("Propiedades: objeto no disponible")?
            .clone();
        let id = self.next_mutation_id();
        self.mutations
            .submit(Request::Inspect {
                id,
                path: path.clone(),
            })
            .map_err(|_| "El worker no aceptó la lectura de propiedades".to_owned())?;
        let mut d = Dialog::new(id, path);
        d.objects = objects;
        d.object_index = index;
        self.session.restore_allowed = false;
        self.operation_message = None;
        self.preview.close();
        self.rename_dialog = None;
        self.settings = false;
        self.actions_menu = false;
        self.notice_open = false;
        self.properties_dialog = Some(d);
        self.hits.clear();
        Ok(())
    }
    pub fn reload_properties(&mut self) {
        if self.mutations.busy || self.mutation_applied {
            return;
        }
        let Some((objects, index)) = self
            .properties_dialog
            .as_ref()
            .map(|d| (d.objects.clone(), d.object_index))
        else {
            return;
        };
        if let Err(error) = self.start_properties(objects, index)
            && let Some(d) = &mut self.properties_dialog
        {
            d.stale = true;
            d.message = error;
        }
    }
    pub fn step_properties(&mut self, delta: i32) {
        if self.mutations.busy || self.mutation_applied || self.copy_busy() {
            return;
        }
        let Some(d) = self.properties_dialog.as_ref() else {
            return;
        };
        let Some(index) = d.step_index(delta) else {
            return;
        };
        let objects = d.objects.clone();
        if let Err(error) = self.start_properties(objects, index)
            && let Some(d) = &mut self.properties_dialog
        {
            d.message = error;
        }
    }
    pub fn apply_properties(&mut self) {
        if self.mutations.busy
            || self.mutation_applied
            || self.properties_dialog.as_ref().is_none_or(|d| !d.changed())
        {
            return;
        }
        self.operation_message = None;
        let id = self.next_mutation_id();
        let d = self.properties_dialog.as_mut().expect("checked dialog");
        let snapshot = d.snapshot.take().expect("editable snapshot");
        let mode = d.requested;
        match self
            .mutations
            .submit(Request::Permissions { id, snapshot, mode })
        {
            Ok(()) => {
                d.id = id;
                d.pending = true;
                d.applying = true;
                d.message = "Aplicando permisos por descriptor…".into();
                d.warnings.clear();
                self.mutation_applied = true;
            }
            Err(request) => {
                if let Request::Permissions { snapshot, .. } = *request {
                    d.snapshot = Some(snapshot);
                }
                d.stale = true;
                d.message = "No se aceptó el cambio; Recargar antes de confirmar".into();
            }
        }
        self.hits.clear();
    }
    pub fn close_properties(&mut self) {
        if self.properties_dialog.as_ref().is_some_and(|d| d.applying) {
            self.mutations.cancel();
            if let Some(d) = &mut self.properties_dialog {
                d.close_after = true;
                d.message = "Cancelación solicitada; esperando confirmación del worker…".into();
            }
        } else {
            if self.properties_dialog.as_ref().is_some_and(|d| d.pending) {
                self.mutations.cancel();
            }
            self.properties_dialog = None;
        }
        self.hits.clear();
    }
    pub fn properties_reply(&mut self, id: u64, reply: Reply) {
        if std::env::var_os("FILES_TEST_PROPERTIES").as_deref() == Some(std::ffi::OsStr::new("1")) {
            eprintln!(
                "FILES_PROPERTIES_ACK id={id} success={} changed={} snapshot={} message={:?}",
                reply.success,
                reply.changed,
                reply.snapshot.is_some(),
                reply.message
            );
        }
        if reply.changed {
            self.can_undo = false;
            self.can_redo = false;
            self.operation_message = Some(reply.message.clone());
        }
        if id == self.mutation_id {
            self.mutation_applied = false;
        }
        let Some(mut d) = self.properties_dialog.take() else {
            return;
        };
        if d.id != id {
            self.properties_dialog = Some(d);
            return;
        }
        if reply.changed {
            let paths = d
                .path
                .parent()
                .into_iter()
                .chain(reply.current_path.as_deref().and_then(|p| p.parent()))
                .map(std::path::Path::to_path_buf)
                .collect::<Vec<_>>();
            let refresh = paths.iter().any(|p| self.path.starts_with(p));
            self.dual.refresh_other(paths.into_iter());
            if refresh {
                self.refresh_current();
            }
        }
        let close = d.close_after;
        d.pending = false;
        d.applying = false;
        let observation = reply
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.properties)
            .or(reply.observed);
        d.stale = !reply.success || observation.is_none();
        if let Some(observed) = observation {
            d.observed = Some(observed);
            if reply.success {
                d.requested = observed.mode & 0o777;
            }
        }
        d.snapshot = reply.snapshot;
        d.current_path = reply.current_path;
        d.message = reply.message;
        d.warnings = reply.warnings;
        if reply.changed && !d.warnings.is_empty() {
            self.closing = false;
            d.close_after = false;
        } else if close {
            return;
        }
        self.properties_dialog = Some(d);
    }
}
pub fn paint(p: &mut dyn crate::painter::Painter, m: &mut Model, w: i32, h: i32) {
    if let Some(mut d) = m.properties_dialog.take() {
        crate::properties_dialog::paint(p, m, &mut d, w, h);
        m.properties_dialog = Some(d);
    }
}
pub fn event(m: &mut Model, ev: Event, h: i32) -> bool {
    match ev {
        Event::Push => m.pointer_down(app::event_x(), app::event_y(), false, false, false),
        Event::MouseWheel => {
            if let Some(d) = &mut m.properties_dialog {
                d.scroll_by(
                    app::event_dy_value().saturating_mul(38),
                    d.body_height.max(1),
                );
            }
        }
        Event::KeyDown | Event::Shortcut => key(m, app::event_key(), app::event_state(), h),
        _ => {}
    }
    true
}

pub fn key(m: &mut Model, key: Key, mods: Shortcut, h: i32) {
    if mods.intersects(Shortcut::Alt) && matches!(key, Key::Left | Key::Right) {
        m.action(Action::PropertyStep(if key == Key::Left { -1 } else { 1 }));
    } else if key == Key::Escape {
        m.action(Action::PropertyClose);
    } else if key == Key::Enter {
        m.action(Action::PropertyApply);
    } else if key == Key::Tab {
        if let Some(d) = &mut m.properties_dialog {
            let height = if d.body_height > 0 {
                d.body_height
            } else {
                ((h - 68).min(640) - 176).max(1)
            };
            d.focus_next(mods.intersects(Shortcut::Shift), height);
        }
    } else if matches!(
        key,
        Key::Up | Key::Down | Key::PageUp | Key::PageDown | Key::Home | Key::End
    ) {
        if let Some(d) = &mut m.properties_dialog {
            let height = d.body_height.max(1);
            let delta = match key {
                Key::Up => -18,
                Key::Down => 18,
                Key::PageUp => -(height - 18).max(18),
                Key::PageDown => (height - 18).max(18),
                Key::Home => i32::MIN,
                _ => i32::MAX,
            };
            d.scroll_by(delta, height);
        }
    } else if key == Key::from_char(' ')
        && let Some(index) = m.properties_dialog.as_ref().map(|d| d.focus)
    {
        m.action(Action::PropertyBit(index));
    }
}

#[cfg(test)]
mod batch_tests {
    use super::*;
    use std::{
        fs::{self, OpenOptions},
        io::Write,
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, Instant},
    };
    struct Fixture {
        root: PathBuf,
        paths: Vec<PathBuf>,
    }
    impl Fixture {
        fn new() -> Self {
            static ID: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "files-properties-batch-{}-{}",
                std::process::id(),
                ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            let paths = vec![root.join("a.txt"), root.join("b.txt")];
            for path in &paths {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .unwrap();
                file.write_all(b"owned properties fixture\n").unwrap();
            }
            Self { root, paths }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            for path in &self.paths {
                let _ = fs::remove_file(path);
            }
            let _ = fs::remove_dir(&self.root);
        }
    }
    fn wait(m: &mut Model, properties: bool) {
        let end = Instant::now() + Duration::from_secs(4);
        while Instant::now() < end {
            m.pump();
            if m.loaded && (!properties || m.properties_dialog.as_ref().is_some_and(|d| !d.pending))
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("owned properties fixture did not settle");
    }
    #[test]
    fn batch_properties_rejects_invalid_whole_requests_before_allocating_or_opening() {
        let mut m = Model::new("/fixture/properties-admission".into());
        m.worker.suspend();
        let id = m.mutation_id;
        for paths in [
            vec![],
            vec!["relative".into()],
            vec!["/valid".into(), "relative".into()],
            (0..33)
                .map(|i| PathBuf::from(format!("/fixture/{i}")))
                .collect(),
            (0..17)
                .map(|i| PathBuf::from(format!("/{i:02}/{}", "x".repeat(4080))))
                .collect(),
        ] {
            assert!(m.open_properties_paths(paths).is_err());
            assert!(m.properties_dialog.is_none());
            assert_eq!(m.mutation_id, id);
        }
        m.editing = true;
        assert!(m.open_properties_paths(vec!["/fixture/a".into()]).is_err());
        assert!(m.properties_dialog.is_none());
        assert_eq!(m.mutation_id, id);
        m.editing = false;
        m.settings = true;
        assert!(m.open_properties_paths(vec!["/fixture/a".into()]).is_err());
        assert!(
            m.settings,
            "external properties do not dismiss an existing menu"
        );
        assert!(m.properties_dialog.is_none());
        assert_eq!(m.mutation_id, id);
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn browser_properties_opens_the_whole_selection_without_changing_it() {
        let f = Fixture::new();
        let mut m = Model::new(f.root.clone());
        wait(&mut m, false);
        m.selection = (0..m.entries.len()).collect();
        let before = (m.path.clone(), m.selection.clone(), m.generation);
        let expected: Vec<_> = m
            .order
            .iter()
            .filter(|i| m.selection.contains(i))
            .map(|i| m.path.join(&m.entries[*i].name))
            .collect();
        m.open_properties();
        wait(&mut m, true);
        assert_eq!(m.properties_dialog.as_ref().unwrap().objects, expected);
        assert_eq!((m.path.clone(), m.selection.clone(), m.generation), before);
        m.close_properties();
        assert!(m.properties_dialog.is_none());
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn batch_properties_reads_each_object_without_navigation_and_preserves_edits() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new();
        let modes: Vec<_> = f
            .paths
            .iter()
            .map(|path| fs::metadata(path).unwrap().permissions().mode())
            .collect();
        let mut m = Model::new(f.root.clone());
        wait(&mut m, false);
        m.view = crate::model::View::List;
        m.sort = files_core::listing::Sort::Name;
        m.query = ".txt".into();
        m.rebuild();
        let browser = (
            m.path.clone(),
            m.generation,
            m.order.clone(),
            m.selection.clone(),
        );
        m.session.restore_allowed = true;
        m.open_properties_paths(vec![
            f.paths[0].clone(),
            f.paths[1].clone(),
            f.paths[0].clone(),
        ])
        .unwrap();
        assert!(!m.session.restore_allowed);
        let first_id = m.properties_dialog.as_ref().unwrap().id;
        m.step_properties(1);
        assert_eq!(
            m.properties_dialog.as_ref().unwrap().id,
            first_id,
            "pending read stays on first object"
        );
        wait(&mut m, true);
        assert_eq!(m.properties_dialog.as_ref().unwrap().objects, f.paths);
        assert!(m.properties_dialog.as_ref().unwrap().editable());
        m.step_properties(1);
        wait(&mut m, true);
        let d = m.properties_dialog.as_ref().unwrap();
        assert_eq!(d.path, f.paths[1]);
        assert_eq!(d.object_index, 1);
        assert_ne!(d.id, first_id);
        let second_id = d.id;
        m.step_properties(1);
        assert_eq!(m.properties_dialog.as_ref().unwrap().id, second_id);
        m.step_properties(-1);
        wait(&mut m, true);
        m.properties_dialog.as_mut().unwrap().toggle(1);
        let d = m.properties_dialog.as_ref().unwrap();
        assert!(d.changed());
        let dirty = (d.id, d.path.clone(), d.requested);
        m.step_properties(1);
        let d = m.properties_dialog.as_ref().unwrap();
        assert_eq!((d.id, d.path.clone(), d.requested), dirty);
        assert!(m.open_properties_paths(vec![f.paths[1].clone()]).is_err());
        m.reload_properties();
        wait(&mut m, true);
        let d = m.properties_dialog.as_ref().unwrap();
        assert_eq!(d.objects, f.paths);
        assert_eq!(d.object_index, 0);
        assert!(!d.changed());
        assert_eq!(
            (
                m.path.clone(),
                m.generation,
                m.order.clone(),
                m.selection.clone()
            ),
            browser
        );
        assert_eq!(m.query, ".txt");
        for (path, mode) in f.paths.iter().zip(modes) {
            assert_eq!(fs::read(path).unwrap(), b"owned properties fixture\n");
            assert_eq!(fs::metadata(path).unwrap().permissions().mode(), mode);
        }
        drop(m);
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn root_observation_is_read_only_and_allows_browsing_to_an_owned_batch_object() {
        use crate::painter::{Image, Painter, Rect};
        use files_core::file_properties::{Identity, Kind, Properties, Timestamp};
        struct Recorder(Vec<String>);
        impl Painter for Recorder {
            fn rect(&mut self, _: Rect, _: files_core::theme::Color) {}
            fn rounded_rect(&mut self, _: Rect, _: i32, _: files_core::theme::Color) {}
            fn text(
                &mut self,
                _: Rect,
                text: &str,
                _: i32,
                _: files_core::theme::Color,
                _: bool,
                _: bool,
            ) {
                self.0.push(text.into());
            }
            fn image(&mut self, _: Rect, _: &Image<'_>) {}
            fn clip(&mut self, _: Rect) {}
            fn unclip(&mut self) {}
        }
        let f = Fixture::new();
        let mut m = Model::new(f.root.clone());
        wait(&mut m, false);
        let browser = (m.path.clone(), m.generation, m.selection.clone());
        let id = m.next_mutation_id();
        let mut d = Dialog::new(id, "/".into());
        d.objects.push(f.paths[0].clone());
        m.properties_dialog = Some(d);
        let time = Timestamp {
            seconds: 1,
            nanoseconds: 0,
        };
        // A synthetic root reply proves UI read-only behavior; this test never
        // opens a host root or requests its permission-change capability.
        let observed = Properties {
            identity: Identity {
                device: 1,
                inode: 2,
                kind: Kind::Directory,
            },
            apparent_bytes: 4096,
            allocated_bytes: 4096,
            hardlinks: 2,
            uid: 0,
            gid: 0,
            mode: 0o755,
            accessed: time,
            modified: time,
            changed: time,
        };
        m.properties_reply(
            id,
            Reply {
                snapshot: None,
                observed: Some(observed),
                current_path: Some("/".into()),
                success: true,
                changed: false,
                message: "Propiedades de la raíz: sólo lectura".into(),
                warnings: vec![],
            },
        );
        let d = m.properties_dialog.as_ref().unwrap();
        assert!(!d.pending && !d.stale && !d.editable() && !d.changed() && !d.has_edits());
        assert!(d.snapshot.is_none());
        assert_eq!(d.observed, Some(observed));
        assert_eq!(d.requested, 0o755);
        assert_eq!(d.step_index(1), Some(1));
        let mut recorder = Recorder(vec![]);
        paint(&mut recorder, &mut m, 1000, 650);
        assert!(
            recorder
                .0
                .iter()
                .any(|text| text.contains("Directorio sólo lectura"))
        );
        assert!(
            !m.hits
                .iter()
                .any(|h| matches!(h.action, Action::PropertyApply | Action::PropertyBit(_)))
        );
        assert!(
            m.hits
                .iter()
                .any(|h| matches!(h.action, Action::PropertyStep(1)))
        );
        m.apply_properties();
        assert!(!m.mutations.busy && !m.mutation_applied);
        m.step_properties(1);
        wait(&mut m, true);
        let d = m.properties_dialog.as_ref().unwrap();
        assert_eq!(d.path, f.paths[0]);
        assert_eq!(d.object_index, 1);
        assert!(d.snapshot.is_some());
        assert_eq!(d.step_index(-1), Some(0));
        assert_eq!((m.path.clone(), m.generation, m.selection.clone()), browser);
        drop(m);
    }
}
