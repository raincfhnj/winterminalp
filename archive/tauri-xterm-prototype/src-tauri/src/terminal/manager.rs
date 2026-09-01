use std::collections::HashMap;
use std::fmt::Display;
use std::io::{Read, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use portable_pty::{Child, ChildKiller, MasterPty, PtySize, native_pty_system};

use super::profile::{BuiltinProfileId, ProfileSpec, list_profiles, resolve_working_directory};
use super::{TerminalError, TerminalEvent, TerminalEventKind, TerminalProfile, TerminalStarted};

const READ_CHUNK_BYTES: usize = 4 * 1024;
const MAX_PTY_DIMENSION: u16 = i16::MAX as u16;
const MAX_PANE_ID_BYTES: usize = 256;

pub type TerminalSink = Arc<dyn Fn(TerminalEvent) + Send + Sync + 'static>;

pub struct TerminalManager {
    sessions: Mutex<HashMap<String, Arc<TerminalSession>>>,
}

impl TerminalManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub fn start_or_attach(
        &self,
        pane_id: &str,
        profile_id: &str,
        cwd: Option<&Path>,
        rows: u16,
        cols: u16,
        sink: TerminalSink,
    ) -> Result<TerminalStarted, TerminalError> {
        validate_pane_id(pane_id)?;
        let size = validate_size(rows, cols)?;
        let requested_profile_id = BuiltinProfileId::parse(profile_id)?;
        let mut sessions = lock(&self.sessions, "terminal manager", None)?;

        if let Some(session) = sessions.get(pane_id)
            && session.is_running()
        {
            if session.launch.profile.id != requested_profile_id {
                return Err(TerminalError::profile_mismatch(
                    pane_id,
                    session.launch.profile.id.as_str(),
                    requested_profile_id.as_str(),
                ));
            }

            session.replace_sink(sink)?;
            session.resize(size)?;
            return Ok(session.started(true));
        }

        let profile = ProfileSpec::from_id(requested_profile_id);
        profile.ensure_available()?;
        let cwd = resolve_working_directory(cwd, &profile);
        let sequence = sessions
            .get(pane_id)
            .map(|session| {
                session.disable_events();
                session.close_streams_best_effort();
                Arc::clone(&session.sequence)
            })
            .unwrap_or_else(|| Arc::new(AtomicU64::new(0)));

        let session = spawn_terminal(pane_id, profile, cwd, size, sequence, sink)?;
        let started = session.started(false);
        sessions.insert(pane_id.to_owned(), session);
        Ok(started)
    }

    pub fn write(&self, pane_id: &str, data: &[u8]) -> Result<(), TerminalError> {
        validate_pane_id(pane_id)?;
        let session = self
            .session(pane_id)?
            .ok_or_else(|| TerminalError::not_running(pane_id))?;
        session.write(data)
    }

    pub fn resize(&self, pane_id: &str, rows: u16, cols: u16) -> Result<(), TerminalError> {
        validate_pane_id(pane_id)?;
        let size = validate_size(rows, cols)?;
        let session = self
            .session(pane_id)?
            .ok_or_else(|| TerminalError::not_running(pane_id))?;
        session.resize(size)
    }

    pub fn terminate(&self, pane_id: &str) -> Result<(), TerminalError> {
        validate_pane_id(pane_id)?;
        if let Some(session) = self.session(pane_id)? {
            session.request_terminate()?;
        }
        Ok(())
    }

    pub fn restart(
        &self,
        pane_id: &str,
        rows: u16,
        cols: u16,
        sink: TerminalSink,
    ) -> Result<TerminalStarted, TerminalError> {
        validate_pane_id(pane_id)?;
        validate_size(rows, cols)?;
        let session = self
            .session(pane_id)?
            .ok_or_else(|| TerminalError::not_running(pane_id))?;
        if session.is_running() {
            return Err(TerminalError::already_running(pane_id));
        }
        let profile_id = session.launch.profile.id.as_str().to_owned();
        let cwd = session.launch.cwd.clone();

        session.disable_events();
        self.start_or_attach(pane_id, &profile_id, cwd.as_deref(), rows, cols, sink)
    }

    pub fn is_running(&self, pane_id: &str) -> Result<bool, TerminalError> {
        validate_pane_id(pane_id)?;
        Ok(self
            .session(pane_id)?
            .is_some_and(|session| session.is_running()))
    }

    #[must_use]
    pub fn list_profiles(&self) -> Vec<TerminalProfile> {
        list_profiles()
    }

    fn session(&self, pane_id: &str) -> Result<Option<Arc<TerminalSession>>, TerminalError> {
        let sessions = lock(&self.sessions, "terminal manager", None)?;
        Ok(sessions.get(pane_id).cloned())
    }
}

impl Default for TerminalManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TerminalManager {
    fn drop(&mut self) {
        let sessions = match self.sessions.get_mut() {
            Ok(sessions) => sessions,
            Err(poisoned) => poisoned.into_inner(),
        };

        for session in sessions.values() {
            session.disable_events();
            session.terminate_best_effort();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TerminalSize {
    rows: u16,
    cols: u16,
}

impl TerminalSize {
    fn as_pty_size(self) -> PtySize {
        PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

#[derive(Debug, Clone)]
struct LaunchConfig {
    profile: ProfileSpec,
    cwd: Option<PathBuf>,
}

struct TerminalSession {
    pane_id: String,
    launch: LaunchConfig,
    process_id: Option<u32>,
    sequence: Arc<AtomicU64>,
    sink: Mutex<TerminalSink>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    killer: Mutex<Option<Box<dyn ChildKiller + Send + Sync>>>,
    size: Mutex<TerminalSize>,
    running: AtomicBool,
    terminating: AtomicBool,
    events_enabled: AtomicBool,
}

struct TerminalSessionParts {
    pane_id: String,
    launch: LaunchConfig,
    process_id: Option<u32>,
    sequence: Arc<AtomicU64>,
    sink: TerminalSink,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    size: TerminalSize,
}

impl TerminalSession {
    fn new(parts: TerminalSessionParts) -> Self {
        Self {
            pane_id: parts.pane_id,
            launch: parts.launch,
            process_id: parts.process_id,
            sequence: parts.sequence,
            sink: Mutex::new(parts.sink),
            writer: Mutex::new(Some(parts.writer)),
            master: Mutex::new(Some(parts.master)),
            killer: Mutex::new(Some(parts.killer)),
            size: Mutex::new(parts.size),
            running: AtomicBool::new(true),
            terminating: AtomicBool::new(false),
            events_enabled: AtomicBool::new(true),
        }
    }

    fn started(&self, attached: bool) -> TerminalStarted {
        TerminalStarted {
            pane_id: self.pane_id.clone(),
            profile_id: self.launch.profile.id.as_str().to_owned(),
            process_id: self.process_id,
            attached,
        }
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    fn replace_sink(&self, sink: TerminalSink) -> Result<(), TerminalError> {
        let mut current = lock(&self.sink, "terminal output sink", Some(&self.pane_id))?;
        *current = sink;
        Ok(())
    }

    fn write(&self, data: &[u8]) -> Result<(), TerminalError> {
        if !self.is_running() {
            return Err(TerminalError::not_running(&self.pane_id));
        }

        let mut writer = lock(&self.writer, "terminal writer", Some(&self.pane_id))?;
        let writer = writer
            .as_mut()
            .ok_or_else(|| TerminalError::not_running(&self.pane_id))?;

        catch_io_operation(Some(&self.pane_id), "write", || {
            writer.write_all(data)?;
            writer.flush()
        })
    }

    fn resize(&self, size: TerminalSize) -> Result<(), TerminalError> {
        if !self.is_running() {
            return Err(TerminalError::not_running(&self.pane_id));
        }

        let master_guard = lock(&self.master, "PTY master", Some(&self.pane_id))?;
        let master = master_guard
            .as_ref()
            .ok_or_else(|| TerminalError::not_running(&self.pane_id))?;
        catch_io_operation(Some(&self.pane_id), "resize", || {
            master.resize(size.as_pty_size())
        })?;
        drop(master_guard);

        let mut current_size = lock(&self.size, "terminal size", Some(&self.pane_id))?;
        *current_size = size;
        Ok(())
    }

    fn request_terminate(&self) -> Result<(), TerminalError> {
        if !self.is_running() || self.terminating.swap(true, Ordering::AcqRel) {
            return Ok(());
        }

        let mut killer = match lock(&self.killer, "terminal process", Some(&self.pane_id)) {
            Ok(killer) => killer,
            Err(error) => {
                self.terminating.store(false, Ordering::Release);
                return Err(error);
            }
        };
        let kill_result = match killer.as_mut() {
            Some(killer) => catch_io_operation(Some(&self.pane_id), "terminate", || killer.kill()),
            None => Ok(()),
        };
        drop(killer);

        if let Err(error) = normalize_kill_result(kill_result) {
            self.terminating.store(false, Ordering::Release);
            return Err(error);
        }

        self.running.store(false, Ordering::Release);
        self.close_streams_best_effort();
        Ok(())
    }

    fn terminate_best_effort(&self) {
        let _ = self.request_terminate();
        self.running.store(false, Ordering::Release);
        self.close_streams_best_effort();
    }

    fn close_streams_best_effort(&self) {
        take_or_recover(&self.writer);
        take_or_recover(&self.master);
    }

    fn finish_process(&self) {
        self.running.store(false, Ordering::Release);
        self.terminating.store(false, Ordering::Release);
        self.close_streams_best_effort();
        take_or_recover(&self.killer);
    }

    fn disable_events(&self) {
        self.events_enabled.store(false, Ordering::Release);
    }

    fn emit(&self, kind: TerminalEventKind, data: Option<String>, exit_code: Option<u32>) {
        if !self.events_enabled.load(Ordering::Acquire) {
            return;
        }

        let sequence = self
            .sequence
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        let sink = match self.sink.lock() {
            Ok(sink) => Arc::clone(&sink),
            Err(poisoned) => Arc::clone(&poisoned.into_inner()),
        };
        let event = TerminalEvent {
            pane_id: self.pane_id.clone(),
            sequence,
            kind,
            data,
            exit_code,
        };

        let _ = panic::catch_unwind(AssertUnwindSafe(|| sink(event)));
    }
}

fn spawn_terminal(
    pane_id: &str,
    profile: ProfileSpec,
    cwd: Option<PathBuf>,
    size: TerminalSize,
    sequence: Arc<AtomicU64>,
    sink: TerminalSink,
) -> Result<Arc<TerminalSession>, TerminalError> {
    let pty_system = native_pty_system();
    let pair = catch_start_operation(Some(pane_id), "create PTY", || {
        pty_system.openpty(size.as_pty_size())
    })?;
    let portable_pty::PtyPair { master, slave } = pair;
    let reader = catch_start_operation(Some(pane_id), "open PTY reader", || {
        master.try_clone_reader()
    })?;
    let writer = catch_start_operation(Some(pane_id), "open PTY writer", || master.take_writer())?;
    let command = profile.command(cwd.as_deref());
    let mut child = catch_start_operation(Some(pane_id), "spawn terminal profile", || {
        slave.spawn_command(command)
    })?;
    drop(slave);

    let process_id = child.process_id();
    let killer = match panic::catch_unwind(AssertUnwindSafe(|| child.clone_killer())) {
        Ok(killer) => killer,
        Err(payload) => {
            let _ = panic::catch_unwind(AssertUnwindSafe(|| child.kill()));
            return Err(TerminalError::start_failed(
                "clone process terminator",
                Some(pane_id),
                panic_message(payload),
            ));
        }
    };

    let session = Arc::new(TerminalSession::new(TerminalSessionParts {
        pane_id: pane_id.to_owned(),
        launch: LaunchConfig { profile, cwd },
        process_id,
        sequence,
        sink,
        writer,
        master,
        killer,
        size,
    }));

    let reader_session = Arc::clone(&session);
    let reader_handle = thread::Builder::new()
        .name("winterminal-pty-reader".to_owned())
        .spawn(move || read_terminal(reader, reader_session))
        .map_err(|error| {
            session.terminate_best_effort();
            TerminalError::worker_failed(pane_id, error.to_string())
        })?;

    let wait_session = Arc::clone(&session);
    thread::Builder::new()
        .name("winterminal-pty-waiter".to_owned())
        .spawn(move || wait_for_terminal(child, reader_handle, wait_session))
        .map_err(|error| {
            session.terminate_best_effort();
            TerminalError::worker_failed(pane_id, error.to_string())
        })?;

    Ok(session)
}

fn read_terminal(mut reader: Box<dyn Read + Send>, session: Arc<TerminalSession>) {
    let mut decoder = Utf8StreamDecoder::default();
    let mut buffer = [0_u8; READ_CHUNK_BYTES];

    loop {
        match reader.read(&mut buffer) {
            Ok(0) => {
                let trailing = decoder.finish();
                if !trailing.is_empty() {
                    session.emit(TerminalEventKind::Output, Some(trailing), None);
                }
                break;
            }
            Ok(read) => {
                let decoded = decoder.push(&buffer[..read]);
                if !decoded.is_empty() {
                    session.emit(TerminalEventKind::Output, Some(decoded), None);
                }
            }
            Err(error) => {
                let trailing = decoder.finish();
                if !trailing.is_empty() {
                    session.emit(TerminalEventKind::Output, Some(trailing), None);
                }
                if session.is_running() && !session.terminating.load(Ordering::Acquire) {
                    session.emit(TerminalEventKind::Error, Some(error.to_string()), None);
                    session.terminate_best_effort();
                }
                break;
            }
        }
    }
}

fn wait_for_terminal(
    mut child: Box<dyn Child + Send + Sync>,
    reader_handle: JoinHandle<()>,
    session: Arc<TerminalSession>,
) {
    let wait_result = panic::catch_unwind(AssertUnwindSafe(|| child.wait()));
    session.finish_process();

    if reader_handle.join().is_err() {
        session.emit(
            TerminalEventKind::Error,
            Some("The terminal output worker stopped unexpectedly.".to_owned()),
            None,
        );
    }

    match wait_result {
        Ok(Ok(status)) => session.emit(
            TerminalEventKind::Exited,
            status.signal().map(str::to_owned),
            Some(status.exit_code()),
        ),
        Ok(Err(error)) => session.emit(TerminalEventKind::Error, Some(error.to_string()), None),
        Err(payload) => session.emit(TerminalEventKind::Error, Some(panic_message(payload)), None),
    }
}

fn validate_pane_id(pane_id: &str) -> Result<(), TerminalError> {
    if pane_id.is_empty() {
        return Err(TerminalError::invalid_pane_id("pane IDs cannot be empty"));
    }
    if pane_id.len() > MAX_PANE_ID_BYTES {
        return Err(TerminalError::invalid_pane_id(format!(
            "pane IDs cannot exceed {MAX_PANE_ID_BYTES} UTF-8 bytes"
        )));
    }
    if pane_id.chars().any(char::is_control) {
        return Err(TerminalError::invalid_pane_id(
            "pane IDs cannot contain control characters",
        ));
    }
    Ok(())
}

fn validate_size(rows: u16, cols: u16) -> Result<TerminalSize, TerminalError> {
    if rows == 0 || cols == 0 || rows > MAX_PTY_DIMENSION || cols > MAX_PTY_DIMENSION {
        return Err(TerminalError::invalid_size(rows, cols, MAX_PTY_DIMENSION));
    }
    Ok(TerminalSize { rows, cols })
}

fn lock<'a, T>(
    mutex: &'a Mutex<T>,
    resource: &str,
    pane_id: Option<&str>,
) -> Result<MutexGuard<'a, T>, TerminalError> {
    mutex
        .lock()
        .map_err(|_| TerminalError::state_unavailable(resource, pane_id))
}

fn take_or_recover<T>(mutex: &Mutex<Option<T>>) {
    match mutex.lock() {
        Ok(mut value) => {
            value.take();
        }
        Err(poisoned) => {
            poisoned.into_inner().take();
        }
    }
}

fn catch_start_operation<T, E, F>(
    pane_id: Option<&str>,
    operation: &'static str,
    action: F,
) -> Result<T, TerminalError>
where
    E: Display,
    F: FnOnce() -> Result<T, E>,
{
    match panic::catch_unwind(AssertUnwindSafe(action)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(TerminalError::start_failed(
            operation,
            pane_id,
            error.to_string(),
        )),
        Err(payload) => Err(TerminalError::start_failed(
            operation,
            pane_id,
            panic_message(payload),
        )),
    }
}

fn catch_io_operation<T, E, F>(
    pane_id: Option<&str>,
    operation: &'static str,
    action: F,
) -> Result<T, TerminalError>
where
    E: Display,
    F: FnOnce() -> Result<T, E>,
{
    match panic::catch_unwind(AssertUnwindSafe(action)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(TerminalError::io_failed(
            operation,
            pane_id,
            error.to_string(),
        )),
        Err(payload) => Err(TerminalError::io_failed(
            operation,
            pane_id,
            panic_message(payload),
        )),
    }
}

#[cfg(windows)]
fn normalize_kill_result(result: Result<(), TerminalError>) -> Result<(), TerminalError> {
    // portable-pty 0.9's Windows ChildKiller reports an error when
    // TerminateProcess succeeds. Closing the ConPTY handles below is the
    // authoritative cancellation step, so the inverted result is ignored.
    match result {
        Err(error) if error.code == "TERMINAL_IO_FAILED" => Ok(()),
        result => result,
    }
}

#[cfg(not(windows))]
fn normalize_kill_result(result: Result<(), TerminalError>) -> Result<(), TerminalError> {
    result
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "a terminal dependency panicked".to_owned()
    }
}

#[derive(Default)]
struct Utf8StreamDecoder {
    pending: Vec<u8>,
}

impl Utf8StreamDecoder {
    fn push(&mut self, bytes: &[u8]) -> String {
        self.decode(bytes, false)
    }

    fn finish(&mut self) -> String {
        self.decode(&[], true)
    }

    fn decode(&mut self, bytes: &[u8], finish: bool) -> String {
        let mut combined = std::mem::take(&mut self.pending);
        combined.extend_from_slice(bytes);
        let mut output = String::new();
        let mut offset = 0;

        while offset < combined.len() {
            match std::str::from_utf8(&combined[offset..]) {
                Ok(valid) => {
                    output.push_str(valid);
                    offset = combined.len();
                }
                Err(error) => {
                    let valid_end = offset + error.valid_up_to();
                    if valid_end > offset {
                        let valid = &combined[offset..valid_end];
                        output.push_str(&String::from_utf8_lossy(valid));
                    }
                    offset = valid_end;

                    match error.error_len() {
                        Some(invalid_len) => {
                            output.push(char::REPLACEMENT_CHARACTER);
                            offset += invalid_len;
                        }
                        None if finish => {
                            output.push_str(&String::from_utf8_lossy(&combined[offset..]));
                            offset = combined.len();
                        }
                        None => {
                            self.pending.extend_from_slice(&combined[offset..]);
                            break;
                        }
                    }
                }
            }
        }

        output
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use std::sync::mpsc;
    #[cfg(windows)]
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn validates_conpty_dimensions_before_the_u16_to_i16_cast() {
        assert_eq!(
            validate_size(24, 80).expect("ordinary terminal dimensions"),
            TerminalSize { rows: 24, cols: 80 }
        );

        for (rows, cols) in [(0, 80), (24, 0), (u16::MAX, 80), (24, u16::MAX)] {
            let error = validate_size(rows, cols).expect_err("invalid dimensions");
            assert_eq!(error.code, "INVALID_TERMINAL_SIZE");
        }
    }

    #[test]
    fn validates_pane_ids_without_restricting_domain_id_format() {
        assert!(validate_pane_id("pane-7d1b9b0e_a").is_ok());
        assert!(validate_pane_id("窗格-1").is_ok());
        assert!(validate_pane_id("").is_err());
        assert!(validate_pane_id("pane\n2").is_err());
        assert!(validate_pane_id(&"p".repeat(MAX_PANE_ID_BYTES + 1)).is_err());
    }

    #[test]
    fn decoder_preserves_utf8_split_across_read_chunks() {
        let text = "PowerShell 中文 🦀";
        let bytes = text.as_bytes();
        let crab_start = text.find('🦀').expect("test string contains emoji");
        let mut decoder = Utf8StreamDecoder::default();

        let first = decoder.push(&bytes[..crab_start + 2]);
        let second = decoder.push(&bytes[crab_start + 2..]);
        let trailing = decoder.finish();

        assert_eq!(format!("{first}{second}{trailing}"), text);
    }

    #[test]
    fn decoder_replaces_invalid_and_incomplete_sequences_without_panicking() {
        let mut decoder = Utf8StreamDecoder::default();

        assert_eq!(decoder.push(b"ok\xFFdone\xE4"), "ok�done");
        assert_eq!(decoder.finish(), "�");
    }

    #[cfg(windows)]
    #[test]
    fn cmd_round_trip_reuses_the_process_when_attaching_again() {
        let manager = TerminalManager::new();
        let (first_tx, first_rx) = mpsc::channel::<TerminalEvent>();
        let first_sink: TerminalSink = Arc::new(move |event| {
            let _ = first_tx.send(event);
        });
        let started = manager
            .start_or_attach("pty-test-pane", "cmd", None, 24, 80, first_sink)
            .expect("Windows CMD should start in ConPTY");

        let (second_tx, second_rx) = mpsc::channel::<TerminalEvent>();
        let second_sink: TerminalSink = Arc::new(move |event| {
            let _ = second_tx.send(event);
        });
        let attached = manager
            .start_or_attach("pty-test-pane", "cmd", None, 30, 100, second_sink)
            .expect("a second start should attach to the existing terminal");

        assert!(!started.attached);
        assert!(attached.attached);
        assert_eq!(started.process_id, attached.process_id);

        let handshake_deadline = Instant::now() + Duration::from_secs(5);
        let mut cursor_query_seen = false;
        while Instant::now() < handshake_deadline && !cursor_query_seen {
            for event in first_rx.try_iter() {
                cursor_query_seen = event.kind == TerminalEventKind::Output
                    && event
                        .data
                        .as_deref()
                        .is_some_and(|data| data.contains("\u{1b}[6n"));
                if cursor_query_seen {
                    break;
                }
            }

            if !cursor_query_seen {
                match second_rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(event) => {
                        cursor_query_seen = event.kind == TerminalEventKind::Output
                            && event
                                .data
                                .as_deref()
                                .is_some_and(|data| data.contains("\u{1b}[6n"));
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        }

        assert!(
            cursor_query_seen,
            "ConPTY should request the terminal cursor position"
        );
        manager
            .write("pty-test-pane", b"\x1b[1;1R")
            .expect("the test terminal should answer ConPTY's cursor query");

        let marker = "WINTERMINAL_PTY_ROUND_TRIP";
        manager
            .write(
                "pty-test-pane",
                format!("echo {marker}\r\nexit\r\n").as_bytes(),
            )
            .expect("CMD input should be written to ConPTY");

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut output = String::new();
        let mut exited = false;
        while Instant::now() < deadline && !exited {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match second_rx.recv_timeout(remaining) {
                Ok(event) => match event.kind {
                    TerminalEventKind::Output => {
                        if let Some(data) = event.data {
                            output.push_str(&data);
                        }
                    }
                    TerminalEventKind::Exited => exited = true,
                    TerminalEventKind::Error => panic!(
                        "terminal worker failed: {}",
                        event.data.as_deref().unwrap_or("unknown terminal error")
                    ),
                },
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        assert!(
            exited,
            "CMD should emit an exited event; output: {output:?}"
        );
        assert!(output.contains(marker), "CMD output: {output:?}");
        assert!(!manager.is_running("pty-test-pane").expect("manager state"));
        manager
            .terminate("pty-test-pane")
            .expect("terminate should be idempotent after exit");
    }
}
