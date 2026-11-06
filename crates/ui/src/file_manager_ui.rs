//! External navigation admission. Bus callbacks and helper work never own Model.
use crate::{
    file_manager_bus::{Server, Status, Ticket},
    file_manager_preflight::Job,
    model::Model,
};
use files_core::file_manager::{Method, Request};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

struct Work {
    id: Option<u64>,
    request: Request,
    job: Job,
    owner: (u64, u64),
    generation: u64,
    started: Instant,
    ticket: Option<Ticket>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::View, tabs::Command};
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let base = std::env::var_os("TMPDIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/var/tmp/files-manager-ui-fixtures"));
            if !base.exists() {
                let mut create = fs::DirBuilder::new();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    create.mode(0o700);
                }
                create.create(&base).unwrap();
            }
            let metadata = fs::symlink_metadata(&base).unwrap();
            assert!(metadata.is_dir() && !metadata.file_type().is_symlink());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
            }
            let root = base.join(format!(
                "manager-ui-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut create = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                create.mode(0o700);
            }
            create.create(&root).unwrap();
            for directory in ["A", "B"] {
                fs::create_dir(root.join(directory)).unwrap();
                for name in ["ordinary.txt", "neighbor.txt", ".hidden"] {
                    fs::write(
                        root.join(directory).join(name),
                        format!("owned {directory}/{name}\n"),
                    )
                    .unwrap();
                }
            }
            // Retain owned fixtures. No cleanup traverses or removes trees.
            Self(root)
        }
        fn model(&self) -> Model {
            let mut m = Model::new(self.0.join("A"));
            // Model-level gestures must not publish fixture preferences or
            // sessions through the real auxiliary persistence worker.
            let (tx, _) = std::sync::mpsc::sync_channel(4);
            let (_, rx) = std::sync::mpsc::channel();
            m.aux = crate::aux_io::AuxIo { tx, rx };
            m.view = View::List;
            m.show_hidden = false;
            ready(&mut m);
            m.rebuild();
            m.config_dirty = false;
            m
        }
    }
    fn ready(m: &mut Model) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            m.pump();
            if m.loaded
                && !m.selection_refreshing()
                && m.dual
                    .other
                    .as_ref()
                    .is_none_or(|b| !m.dual.enabled || b.loaded && b.refresh_selection.is_none())
            {
                return;
            }
            assert!(Instant::now() < deadline, "{}", m.error);
            std::thread::yield_now();
        }
    }
    fn select(m: &mut Model, name: &str) {
        let index = m.entries.iter().position(|e| e.name == name).unwrap();
        m.select_entry(index, false, false);
    }
    fn group(path: PathBuf, names: &[&str]) -> files_core::file_manager::Group {
        files_core::file_manager::Group {
            directory: path,
            names: names.iter().map(OsString::from).collect(),
        }
    }
    fn selection(m: &Model) -> Vec<OsString> {
        let mut names = m
            .selection
            .iter()
            .map(|&i| m.entries[i].name.clone())
            .collect::<Vec<_>>();
        names.sort();
        names
    }
    #[test]
    fn missing_exact_external_name_clears_all_marks_and_never_selects_a_neighbor() {
        let f = Fixture::new();
        let mut m = f.model();
        select(&mut m, "neighbor.txt");
        let path = m.path.clone();
        let owner = m.pane_owner();
        let generation = m.generation;
        m.external_reveal = Some(Reveal {
            owner,
            generation,
            names: vec!["ordinary.txt".into(), "missing.txt".into()],
        });
        assert!(Controller::finish_reveal(&mut m));
        assert!(m.selection.is_empty() && m.selected.is_none() && m.anchor.is_none());
        assert!(m.preserve_empty_selection);
        assert!(m.external_reveal.is_none());
        assert!(!m.error.is_empty());
        assert_eq!(m.path, path);
        assert_eq!(m.pane_owner(), owner);
        assert_eq!(m.generation, generation);
        assert!(
            m.entries.iter().any(|e| e.name == "ordinary.txt")
                && m.entries.iter().any(|e| e.name == "neighbor.txt")
        );
    }
    #[test]
    fn same_location_refresh_preserves_the_complete_pending_external_selection() {
        let f = Fixture::new();
        let mut m = f.model();
        m.external_locations(
            &[group(f.0.join("A"), &["ordinary.txt", "missing.txt"])],
            false,
        )
        .unwrap();
        ready(&mut m);
        // This is the point where the listing watcher can request a refresh:
        // ordinary Snapshot restoration has found only one requested name,
        // and the external all-or-nothing check has not run yet.
        assert_eq!(selection(&m), [OsString::from("ordinary.txt")]);
        let owner = m.pane_owner();
        let generation = m.generation;
        m.refresh_current();
        assert_eq!(m.pane_owner(), owner);
        assert_ne!(m.generation, generation);
        assert_eq!(m.external_reveal.as_ref().unwrap().generation, m.generation);
        assert_eq!(
            m.external_reveal.as_ref().unwrap().names,
            [
                OsString::from("ordinary.txt"),
                OsString::from("missing.txt")
            ]
        );
        ready(&mut m);
        assert!(Controller::finish_reveal(&mut m));
        assert!(m.selection.is_empty() && m.selected.is_none() && m.anchor.is_none());
        assert!(m.preserve_empty_selection && !m.error.is_empty());
    }
    #[test]
    fn stale_external_owner_or_generation_does_not_replace_current_selection() {
        let f = Fixture::new();
        let mut m = f.model();
        select(&mut m, "neighbor.txt");
        let before = (selection(&m), m.selected, m.anchor, m.error.clone());
        for (owner, generation) in [
            ((m.dual.active_id + 1, m.tabs.active), m.generation),
            (m.pane_owner(), m.generation.wrapping_add(1)),
        ] {
            m.external_reveal = Some(Reveal {
                owner,
                generation,
                names: vec!["ordinary.txt".into()],
            });
            assert!(!Controller::finish_reveal(&mut m));
            assert!(m.external_reveal.is_none());
            assert_eq!(
                before,
                (selection(&m), m.selected, m.anchor, m.error.clone())
            );
        }
    }
    #[test]
    fn transient_hidden_reveal_survives_tab_restoration_without_changing_preferences() {
        let f = Fixture::new();
        let mut m = f.model();
        let preference = m.show_hidden;
        let dirty = m.config_dirty;
        let ids = m
            .external_locations(
                &[
                    group(f.0.join("B"), &[".hidden"]),
                    group(f.0.join("A"), &["ordinary.txt"]),
                ],
                false,
            )
            .unwrap();
        ready(&mut m);
        assert!(Controller::finish_reveal(&mut m));
        assert_eq!(selection(&m), [OsString::from(".hidden")]);
        assert!(m.reveal_hidden && m.hidden_visible());
        assert_eq!(m.show_hidden, preference);
        assert_eq!(m.config_dirty, dirty);
        m.tab_command(Command::Select(ids[1]));
        ready(&mut m);
        assert!(Controller::finish_reveal(&mut m));
        assert!(!m.reveal_hidden);
        assert!(!m.order.iter().any(|&i| m.entries[i].name == ".hidden"));
        m.tab_command(Command::Select(ids[0]));
        ready(&mut m);
        assert!(m.reveal_hidden);
        assert_eq!(selection(&m), [OsString::from(".hidden")]);
        assert_eq!(m.show_hidden, preference);
        assert_eq!(m.config_dirty, dirty);
        m.navigate(f.0.join("A"));
        ready(&mut m);
        assert!(!m.reveal_hidden && !m.hidden_visible());
        assert_eq!(m.show_hidden, preference);
        assert_eq!(m.config_dirty, dirty);
    }
    #[test]
    fn pending_external_reveal_follows_its_dual_pane_and_rebinds_generation_on_return() {
        let f = Fixture::new();
        let mut m = f.model();
        m.toggle_dual();
        ready(&mut m);
        let left = m.pane_owner();
        let preference = m.show_hidden;
        let dirty = m.config_dirty;
        m.external_locations(&[group(f.0.join("A"), &[".hidden"])], false)
            .unwrap();
        let old_generation = m.generation;
        assert!(m.external_reveal.is_some());
        m.switch_dual();
        assert_ne!(m.pane_owner().0, left.0);
        assert!(m.external_reveal.is_none());
        assert!(!m.reveal_hidden);
        let pending = m
            .dual
            .other
            .as_ref()
            .unwrap()
            .external_reveal
            .as_ref()
            .unwrap();
        assert_eq!(pending.owner, left);
        assert_eq!(pending.generation, old_generation);
        assert_eq!(pending.names, [OsString::from(".hidden")]);
        let right = (selection(&m), m.selected, m.anchor);
        assert!(!Controller::finish_reveal(&mut m));
        assert_eq!(right, (selection(&m), m.selected, m.anchor));
        m.switch_dual();
        assert_eq!(m.pane_owner(), left);
        assert!(m.reveal_hidden);
        let pending = m.external_reveal.as_ref().unwrap();
        assert_eq!(pending.owner, left);
        assert_eq!(pending.generation, m.generation);
        assert_ne!(m.generation, old_generation);
        ready(&mut m);
        assert!(Controller::finish_reveal(&mut m));
        assert_eq!(selection(&m), [OsString::from(".hidden")]);
        assert!(m.external_reveal.is_none());
        assert_eq!(m.show_hidden, preference);
        assert_eq!(m.config_dirty, dirty);
    }
    #[test]
    fn explicit_visibility_or_filter_change_cannot_restore_an_invisible_requested_mark() {
        let f = Fixture::new();
        for hidden in [false, true] {
            let mut m = f.model();
            let name = if hidden { ".hidden" } else { "ordinary.txt" };
            m.external_locations(&[group(f.0.join("A"), &[name])], false)
                .unwrap();
            if hidden {
                m.toggle_hidden();
                m.toggle_hidden();
            } else {
                crate::input(&mut m, "no-owned-match");
            }
            ready(&mut m);
            assert!(!m.order.iter().any(|&i| m.entries[i].name == name));
            assert!(Controller::finish_reveal(&mut m));
            assert!(m.selection.is_empty() && m.selected.is_none() && m.anchor.is_none());
            assert!(m.preserve_empty_selection);
            assert!(!m.error.is_empty());
        }
    }
    #[test]
    fn protected_cli_admission_starts_no_worker_or_bus_owner() {
        let f = Fixture::new();
        let mut m = f.model();
        let mut controller = Controller::new(false).unwrap();
        let request =
            Request::from_paths(Method::ShowItems, vec![f.0.join("A/ordinary.txt")]).unwrap();
        m.help = Some(Default::default());
        assert!(controller.enqueue_cli(&m, request.clone()).is_err());
        m.help = None;
        m.editing = true;
        assert!(controller.enqueue_cli(&m, request).is_err());
        assert!(controller.work.is_none() && controller.retired.is_empty());
        assert!(controller.drained());
        assert_eq!(controller.status(), "disabled");
        assert_eq!(controller.accepted, 0);
    }
    #[test]
    fn raw_cli_parent_components_are_preserved_for_kernel_symlink_resolution() {
        let cwd = Path::new("/owned/working-directory");
        let operands = [PathBuf::from("alias/../item%25"), PathBuf::from("./other")];
        let request = cli_request(Method::ShowItems, &operands, cwd).unwrap();
        assert_eq!(
            request.paths[0].as_os_str(),
            cwd.join("alias/../item%25").as_os_str()
        );
        assert_eq!(
            request.paths[1].as_os_str(),
            cwd.join("./other").as_os_str()
        );
        let uris = cli_request(
            Method::ShowItems,
            &[PathBuf::from("FILE://LOCALHOST/owned/item%2525")],
            cwd,
        )
        .unwrap();
        assert_eq!(uris.paths[0], PathBuf::from("/owned/item%25"));
    }
}
pub struct Reveal {
    pub owner: (u64, u64),
    pub generation: u64,
    pub names: Vec<OsString>,
}
impl Reveal {
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.names.capacity() * std::mem::size_of::<OsString>()
            + self.names.iter().map(|n| n.capacity()).sum::<usize>()
    }
}
#[derive(Default)]
pub struct Effects {
    pub dirty: bool,
    pub present: bool,
    pub fatal: Option<String>,
}
pub struct Controller {
    server: Option<Server>,
    work: Option<Work>,
    retired: Vec<Job>,
    fresh: bool,
    required: bool,
    pub accepted: u64,
    pub rejected: u64,
    pub last_error: String,
}
pub fn available(m: &Model) -> bool {
    !crate::tab_gestures::blocked(m) && !m.copy_busy() && !m.mutation_applied
}
pub fn cli_request(method: Method, operands: &[PathBuf], cwd: &Path) -> Result<Request, String> {
    if operands.is_empty() || operands.len() > files_core::file_manager::MAX_URIS {
        return Err("Indica entre 1 y 32 ubicaciones".into());
    }
    let mut paths = Vec::new();
    for operand in operands {
        if let Some(uri) = operand.to_str().filter(|s| {
            s.get(..7)
                .is_some_and(|p| p.eq_ignore_ascii_case("file://"))
        }) {
            paths.extend(Request::from_uris(method, &[uri.to_owned()], "")?.paths);
        } else {
            paths.push(if operand.is_absolute() {
                operand.clone()
            } else {
                cwd.join(operand)
            });
        }
    }
    Request::from_paths(method, paths)
}
impl Controller {
    pub fn new(service: bool) -> Result<Self, String> {
        Ok(Self {
            server: if service {
                Some(Server::start(true)?)
            } else {
                None
            },
            work: None,
            retired: Vec::new(),
            fresh: true,
            required: service,
            accepted: 0,
            rejected: 0,
            last_error: String::new(),
        })
    }
    pub fn status(&self) -> String {
        match self.server.as_ref().map(Server::status) {
            None => "disabled".into(),
            Some(Status::Starting) => "starting".into(),
            Some(Status::Owned) => "owned".into(),
            Some(Status::Unavailable(e)) => format!("unavailable: {e}"),
        }
    }
    pub fn enqueue_cli(&mut self, m: &Model, request: Request) -> Result<(), String> {
        self.begin(m, None, None, request)
    }
    fn begin(
        &mut self,
        m: &Model,
        id: Option<u64>,
        ticket: Option<Ticket>,
        request: Request,
    ) -> Result<(), String> {
        if self.work.is_some() || self.retired.len() >= 8 || !available(m) {
            return Err("Termina o cancela la edición, el diálogo o la operación antes de mostrar otra ubicación".into());
        }
        let job = Job::start(&request)?;
        self.work = Some(Work {
            id,
            request,
            job,
            owner: m.pane_owner(),
            generation: m.generation,
            started: Instant::now(),
            ticket,
        });
        Ok(())
    }
    fn reply(&mut self, id: Option<u64>, result: Result<(), String>) {
        if let Err(error) = &result {
            self.rejected += 1;
            self.last_error = error.clone();
        } else {
            self.accepted += 1;
            self.last_error.clear();
        }
        if let (Some(server), Some(id)) = (&self.server, id) {
            server.complete(id, result);
        }
    }
    pub fn pump(&mut self, m: &mut Model) -> Effects {
        let mut effects = Effects::default();
        self.retired.retain(|job| !job.finished());
        if m.closing {
            self.stop();
            return effects;
        }
        if let Some(Status::Unavailable(error)) = self.server.as_ref().map(Server::status)
            && self.required
            && self.fresh
        {
            effects.fatal = Some(error);
            return effects;
        }
        if self.work.as_ref().is_some_and(|w| {
            w.owner != m.pane_owner()
                || w.generation != m.generation
                || !available(m)
                || w.ticket.as_ref().is_some_and(|t| !t.is_live())
        }) {
            let work = self.work.take().expect("stale request");
            work.job.cancel();
            let error =
                "La vista cambió mientras se comprobaba la ubicación; solicita mostrarla otra vez"
                    .to_owned();
            self.reply(work.id, Err(error.clone()));
            self.retired.push(work.job);
            if work.id.is_none() {
                effects.fatal = Some(error);
                return effects;
            }
        }
        while let Some(event) = self.server.as_ref().and_then(Server::poll) {
            let id = event.id;
            let ticket = event.ticket.clone();
            if let Err(error) = self.begin(m, Some(id), Some(ticket), event.request) {
                self.reply(Some(id), Err(error));
            }
        }
        let result = self.work.as_mut().and_then(|w| w.job.poll());
        if let Some(mut result) = result {
            let work = self.work.take().expect("completed preflight");
            if result.is_ok()
                && (work.started.elapsed() > Duration::from_secs(4)
                    || work.owner != m.pane_owner()
                    || work.generation != m.generation
                    || work.ticket.as_ref().is_some_and(|ticket| !ticket.is_live())
                    || !available(m))
            {
                result=Err("La vista cambió mientras se comprobaba la ubicación; solicita mostrarla otra vez".into());
            }
            if result.is_ok() {
                result = match work.request.method {
                    Method::ShowItemProperties => m.open_properties_paths(work.request.paths),
                    Method::ShowItems | Method::ShowFolders => {
                        let groups = work.request.groups();
                        m.external_locations(&groups, self.fresh).map(|_| {
                            m.external_reveal = Some(Reveal {
                                owner: m.pane_owner(),
                                generation: m.generation,
                                names: groups[0].names.clone(),
                            });
                        })
                    }
                };
            }
            let accepted = result.is_ok();
            self.reply(work.id, result);
            if !work.job.finished() {
                self.retired.push(work.job);
            }
            effects.dirty = true;
            if accepted {
                self.fresh = false;
                effects.present = true;
            } else if work.id.is_none() {
                effects.fatal = Some(self.last_error.clone());
            }
        }
        effects
    }
    pub fn stop(&mut self) {
        if let Some(server) = &self.server {
            server.stop();
        }
        if let Some(work) = self.work.take() {
            work.job.cancel();
            self.retired.push(work.job);
        }
        for job in &self.retired {
            job.cancel();
        }
    }
    pub fn drained(&self) -> bool {
        self.work.is_none()
            && self.retired.iter().all(Job::drained)
            && self.server.as_ref().is_none_or(Server::drained)
    }
    pub fn finish_reveal(m: &mut Model) -> bool {
        let Some(reveal) = m.external_reveal.as_ref() else {
            return false;
        };
        if reveal.owner != m.pane_owner() || reveal.generation != m.generation {
            m.external_reveal = None;
            return false;
        }
        if !m.loaded || m.selection_refreshing() {
            return false;
        }
        let reveal = m.external_reveal.take().expect("ready reveal");
        let found: Vec<_> = reveal
            .names
            .iter()
            .filter_map(|name| {
                m.entries
                    .iter()
                    .position(|e| e.name == *name)
                    .filter(|index| m.order.contains(index))
            })
            .collect();
        if found.len() != reveal.names.len() {
            m.selection.clear();
            m.selected = None;
            m.anchor = None;
            m.preserve_empty_selection = true;
            m.error="Uno de los archivos solicitados ya no está disponible en esta vista; no se seleccionó otro objeto".into();
        } else {
            let focus = m
                .selected
                .filter(|i| found.contains(i))
                .or_else(|| found.first().copied());
            m.selection = found.iter().copied().collect();
            m.selected = focus;
            m.anchor = m.selected;
            m.preserve_empty_selection = found.is_empty();
            m.search_reveal = !found.is_empty();
        }
        true
    }
    pub fn marker(&self, m: &Model) -> serde_json::Value {
        let hex = |name: &std::ffi::OsStr| {
            name.as_encoded_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        let tabs = |tabs: &crate::tabs::Tabs| {
            tabs.rows
                .iter()
                .map(|t| serde_json::json!({"id":t.id,"path_raw":hex(t.path.as_os_str())}))
                .collect::<Vec<_>>()
        };
        let mut value = serde_json::json!({"path_raw":hex(m.path.as_os_str()),"active":m.tabs.active,
            "pane_id":m.dual.active_id,"tabs":tabs(&m.tabs),"generation":m.generation,
            "selection_raw":m.selection.iter().filter_map(|i|m.entries.get(*i)).map(|e|hex(&e.name)).collect::<Vec<_>>(),
            "selected_raw":m.selected.and_then(|i|m.entries.get(i)).map(|e|hex(&e.name)),
            "anchor_raw":m.anchor.and_then(|i|m.entries.get(i)).map(|e|hex(&e.name)),
            "loaded":m.loaded,"settled":m.listing_sync.settled(),"refreshing":m.selection_refreshing(),
            "pending_view":m.external_reveal.is_some(),"closing":m.closing,"error":m.error,
            "config_dirty":m.config_dirty,"modal":crate::places_sidebar::covered(m),
            "other_tabs":m.dual.other.as_ref().map(|b|tabs(&b.tabs)),"view":match m.view{
                crate::model::View::List=>"list",crate::model::View::Grid=>"grid",crate::model::View::Groups=>"groups",
                crate::model::View::Columns=>"columns",crate::model::View::SpaceMap=>"space-map",crate::model::View::Spatial=>"spatial"}});
        value["service_status"] = self.status().into();
        value["accepted"] = self.accepted.into();
        value["rejected"] = self.rejected.into();
        value["request_error"] = self.last_error.clone().into();
        value["pending"] = self.work.is_some().into();
        value["properties"] = crate::properties_dialog::fixture(m);
        value["show_hidden_preference"] = m.show_hidden.into();
        value["reveal_hidden"] = m.reveal_hidden.into();
        value
    }
}
