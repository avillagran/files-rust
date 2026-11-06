//! Optional FileManager1 service. Typed D-Bus decoding stays on its worker;
//! the UI brokers bounded helper preflight and admission before method replies.
#[cfg(target_os = "linux")]
use files_core::file_manager::Method;
use files_core::file_manager::Request;
#[cfg(target_os = "linux")]
use std::{
    collections::BTreeMap,
    ffi::{CStr, CString, c_char, c_int, c_void},
    time::Duration,
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::JoinHandle,
    time::Instant,
};

const PENDING: usize = 8;
#[cfg(target_os = "linux")]
const ADMISSION: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Starting,
    Owned,
    Unavailable(String),
}
pub struct Event {
    pub id: u64,
    pub request: Request,
    pub ticket: Ticket,
}
#[derive(Clone)]
pub struct Ticket {
    deadline: Instant,
    live: Arc<AtomicBool>,
}
impl Ticket {
    pub fn is_live(&self) -> bool {
        self.live.load(Ordering::Acquire) && Instant::now() < self.deadline
    }
}
struct Ack {
    id: u64,
    result: Result<(), String>,
}
pub struct Server {
    status: Arc<Mutex<Status>>,
    incoming: Receiver<Event>,
    complete: SyncSender<Ack>,
    stop: Arc<AtomicBool>,
    drained: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Server {
    pub fn start(required: bool) -> Result<Self, String> {
        let (send, incoming) = mpsc::sync_channel(PENDING);
        let (complete, acknowledgements) = mpsc::sync_channel(PENDING * 2);
        let status = Arc::new(Mutex::new(Status::Starting));
        let stop = Arc::new(AtomicBool::new(false));
        let drained = Arc::new(AtomicBool::new(false));
        let address = std::env::var("DBUS_SESSION_BUS_ADDRESS")
            .map_err(|_| "D-Bus session address is unavailable".to_string())
            .and_then(|address| {
                // Never trigger libdbus autolaunch or an external TCP endpoint.
                if address.is_empty()
                    || address.len() > 4096
                    || address.contains('\0')
                    || address.split(';').any(|part| !part.starts_with("unix:"))
                {
                    return Err("FileManager1 requires an explicit local Unix session bus".into());
                }
                Ok(address)
            });
        let address = match address {
            Ok(address) => address,
            Err(error) if required => return Err(error),
            Err(error) => {
                *status.lock().unwrap_or_else(|e| e.into_inner()) = Status::Unavailable(error);
                drained.store(true, Ordering::Release);
                return Ok(Self {
                    status,
                    incoming,
                    complete,
                    stop,
                    drained,
                    worker: None,
                });
            }
        };
        let (state, stopping, exited) = (status.clone(), stop.clone(), drained.clone());
        let worker = std::thread::Builder::new()
            .name("files-filemanager-bus".into())
            .spawn(move || {
                let result = serve(&address, &stopping, &send, &acknowledgements, &state);
                if let Err(error) = result {
                    *state.lock().unwrap_or_else(|e| e.into_inner()) = Status::Unavailable(error);
                }
                exited.store(true, Ordering::Release);
            })
            .map_err(|e| format!("Cannot start FileManager1 worker: {e}"))?;
        Ok(Self {
            status,
            incoming,
            complete,
            stop,
            drained,
            worker: Some(worker),
        })
    }
    pub fn status(&self) -> Status {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn poll(&self) -> Option<Event> {
        while let Ok(event) = self.incoming.try_recv() {
            if event.ticket.is_live() {
                return Some(event);
            }
        }
        None
    }
    pub fn complete(&self, id: u64, result: Result<(), String>) -> bool {
        self.complete.try_send(Ack { id, result }).is_ok()
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
    pub fn drained(&self) -> bool {
        self.drained.load(Ordering::Acquire)
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
        // Established-bus admission has a deadline; initial libdbus connection
        // setup does not. Never join a live worker from a UI destructor.
        if self.worker.as_ref().is_some_and(|w| w.is_finished()) {
            let _ = self.worker.take().expect("finished worker").join();
        }
    }
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn files_fm_open(address: *const c_char, reason: *mut c_char, capacity: usize) -> *mut c_void;
    fn files_fm_close(bus: *mut c_void);
    fn files_fm_next(
        bus: *mut c_void,
        wait: c_int,
        out: *mut *mut c_void,
        reason: *mut c_char,
        capacity: usize,
    ) -> c_int;
    fn files_fm_method(call: *const c_void) -> c_int;
    fn files_fm_count(call: *const c_void) -> usize;
    fn files_fm_uri(call: *const c_void, index: usize) -> *const c_char;
    fn files_fm_startup(call: *const c_void) -> *const c_char;
    fn files_fm_reply(
        bus: *mut c_void,
        call: *mut c_void,
        name: *const c_char,
        text: *const c_char,
    ) -> c_int;
    fn files_fm_free_call(call: *mut c_void);
}

#[cfg(target_os = "linux")]
struct Bus(*mut c_void);
#[cfg(target_os = "linux")]
impl Drop for Bus {
    fn drop(&mut self) {
        unsafe {
            files_fm_close(self.0);
        }
    }
}
#[cfg(target_os = "linux")]
struct Call(*mut c_void);
#[cfg(target_os = "linux")]
impl Drop for Call {
    fn drop(&mut self) {
        unsafe {
            files_fm_free_call(self.0);
        }
    }
}
#[cfg(target_os = "linux")]
impl Call {
    fn request(&self) -> Result<Request, String> {
        // libdbus validated UTF-8 and the exact `ass` signature; the C bridge
        // bounded every string/count before returning these borrowed pointers.
        unsafe {
            let method = match files_fm_method(self.0) {
                0 => Method::ShowFolders,
                1 => Method::ShowItems,
                2 => Method::ShowItemProperties,
                _ => return Err("Unknown FileManager1 method".into()),
            };
            let count = files_fm_count(self.0);
            if count > 32 {
                return Err("Too many FileManager1 URIs".into());
            }
            let mut uris = Vec::with_capacity(count);
            for i in 0..count {
                let p = files_fm_uri(self.0, i);
                if p.is_null() {
                    return Err("Missing FileManager1 URI".into());
                }
                uris.push(
                    CStr::from_ptr(p)
                        .to_str()
                        .map_err(|_| "Invalid URI UTF-8")?
                        .to_string(),
                );
            }
            let startup = files_fm_startup(self.0);
            if startup.is_null() {
                return Err("Missing startup ID".into());
            }
            Request::from_uris(
                method,
                &uris,
                CStr::from_ptr(startup)
                    .to_str()
                    .map_err(|_| "Invalid startup ID UTF-8")?,
            )
        }
    }
    fn reply(mut self, bus: &Bus, error: Option<(&str, &str)>) -> Result<(), String> {
        let (name, text) = if let Some((name, text)) = error {
            let mut text = text.replace('\0', "?");
            if text.len() > 2048 {
                let mut end = 2048;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                text.truncate(end);
            }
            (
                Some(CString::new(name).map_err(|_| "Invalid D-Bus error name")?),
                Some(CString::new(text).map_err(|_| "Invalid D-Bus error text")?),
            )
        } else {
            (None, None)
        };
        let ok = unsafe {
            files_fm_reply(
                bus.0,
                self.0,
                name.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
                text.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            )
        };
        self.0 = std::ptr::null_mut();
        if ok == 0 {
            Err("Cannot queue FileManager1 reply".into())
        } else {
            Ok(())
        }
    }
}
#[cfg(target_os = "linux")]
struct Pending {
    call: Call,
    ticket: Ticket,
}
#[cfg(target_os = "linux")]
impl Drop for Pending {
    fn drop(&mut self) {
        self.ticket.live.store(false, Ordering::Release);
    }
}

#[cfg(target_os = "linux")]
fn serve(
    address: &str,
    stop: &AtomicBool,
    incoming: &SyncSender<Event>,
    acks: &Receiver<Ack>,
    status: &Mutex<Status>,
) -> Result<(), String> {
    let address = CString::new(address).map_err(|_| "Invalid D-Bus address")?;
    let mut reason = [0 as c_char; 1024];
    let ptr = unsafe { files_fm_open(address.as_ptr(), reason.as_mut_ptr(), reason.len()) };
    if ptr.is_null() {
        return Err(unsafe { CStr::from_ptr(reason.as_ptr()) }
            .to_string_lossy()
            .into_owned());
    }
    let bus = Bus(ptr);
    *status.lock().unwrap_or_else(|e| e.into_inner()) = Status::Owned;
    let mut pending: BTreeMap<u64, Pending> = BTreeMap::new();
    let mut next = 1u64;
    while !stop.load(Ordering::Acquire) {
        while let Ok(ack) = acks.try_recv() {
            if let Some(mut item) = pending.remove(&ack.id) {
                let call = std::mem::replace(&mut item.call, Call(std::ptr::null_mut()));
                call.reply(
                    &bus,
                    ack.result
                        .as_ref()
                        .err()
                        .map(|text| ("org.freedesktop.DBus.Error.Failed", text.as_str())),
                )?;
            }
        }
        let expired: Vec<_> = pending
            .iter()
            .filter_map(|(&id, p)| (!p.ticket.is_live()).then_some(id))
            .collect();
        for id in expired {
            let mut item = pending.remove(&id).expect("expired pending call");
            let call = std::mem::replace(&mut item.call, Call(std::ptr::null_mut()));
            call.reply(
                &bus,
                Some((
                    "org.freedesktop.DBus.Error.TimedOut",
                    "FileManager1 UI admission timed out",
                )),
            )?;
        }
        let mut call = std::ptr::null_mut();
        match unsafe { files_fm_next(bus.0, 25, &mut call, reason.as_mut_ptr(), reason.len()) } {
            0 => continue,
            1 => {}
            _ => {
                return Err(unsafe { CStr::from_ptr(reason.as_ptr()) }
                    .to_string_lossy()
                    .into_owned());
            }
        }
        let call = Call(call);
        if pending.len() >= PENDING {
            call.reply(
                &bus,
                Some((
                    "org.freedesktop.DBus.Error.LimitsExceeded",
                    "FileManager1 has too many pending requests",
                )),
            )?;
            continue;
        }
        let request = match call.request() {
            Ok(request) => request,
            Err(error) => {
                call.reply(
                    &bus,
                    Some(("org.freedesktop.DBus.Error.InvalidArgs", &error)),
                )?;
                continue;
            }
        };
        if stop.load(Ordering::Acquire) {
            break;
        }
        let id = next;
        next = next
            .checked_add(1)
            .ok_or("FileManager1 request IDs exhausted")?;
        let deadline = Instant::now() + ADMISSION;
        let live = Arc::new(AtomicBool::new(true));
        let ticket = Ticket {
            deadline,
            live: live.clone(),
        };
        if incoming
            .try_send(Event {
                id,
                request,
                ticket: ticket.clone(),
            })
            .is_err()
        {
            call.reply(
                &bus,
                Some((
                    "org.freedesktop.DBus.Error.LimitsExceeded",
                    "FileManager1 UI queue is full",
                )),
            )?;
            continue;
        }
        pending.insert(id, Pending { call, ticket });
    }
    // Closing the private connection releases the name and completes any
    // outstanding callers with disconnect; no shared bus handle is touched.
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn serve(
    _: &str,
    _: &AtomicBool,
    _: &SyncSender<Event>,
    _: &Receiver<Ack>,
    _: &Mutex<Status>,
) -> Result<(), String> {
    Err("FileManager1 service is available on Linux only".into())
}
