//! Background jobs ([§12.4]): long work runs on a pool of threads, the TUI
//! reads job state through a channel and stays responsive, `Ctrl-C`/`Esc`
//! cancels at a script boundary and the job reports what it did not run.

use crate::config::Config;
use crate::db::Db;
use crate::error::Result;
use crate::model::{Progress, Scope};
use crate::scripts::RunOutcome;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

#[derive(Debug)]
pub enum JobMsg {
    Progress(Progress),
    Log(String),
    ScanDone(Box<crate::scan::LibraryModel>),
    ScriptDone(Box<RunOutcome>),
    LayoutDone(Box<Vec<crate::layout::ApplyOutcome>>),
    Error(String),
    Finished,
}

#[derive(Debug, Clone)]
pub enum JobRequest {
    Scan(Scope),
    Script { id: u8, scope: Scope },
    LayoutApply { ids: Option<Vec<String>>, dry_run: bool },
}

impl JobRequest {
    pub fn kind(&self) -> String {
        match self {
            JobRequest::Scan(_) => "scan".into(),
            JobRequest::Script { id, .. } => format!("script {id}"),
            JobRequest::LayoutApply { .. } => "layout apply".into(),
        }
    }
    pub fn scope(&self) -> Scope {
        match self {
            JobRequest::Scan(s) => s.clone(),
            JobRequest::Script { scope, .. } => scope.clone(),
            JobRequest::LayoutApply { .. } => Scope::Library,
        }
    }
}

pub struct JobHandle {
    pub id: u64,
    pub kind: String,
    pub scope_label: String,
    pub rx: Receiver<JobMsg>,
    pub cancel: Arc<AtomicBool>,
    pub done: Arc<AtomicBool>,
}

impl JobHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Relaxed)
    }
    /// Drain everything currently available (non-blocking).
    pub fn drain(&self) -> Vec<JobMsg> {
        let mut out = Vec::new();
        while let Ok(m) = self.rx.try_recv() {
            out.push(m);
        }
        out
    }
}

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Spawn a job on its own thread with its own SQLite connection.
pub fn spawn(req: JobRequest, cfg: Config) -> Result<JobHandle> {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let kind = req.kind();
    let scope_label = req.scope().label();

    let cancel_t = cancel.clone();
    let done_t = done.clone();
    std::thread::Builder::new()
        .name(format!("mlo-job-{id}"))
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_job(req, &cfg, &tx, &cancel_t)
            }));
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    let _ = tx.send(JobMsg::Error(e.to_string()));
                    let _ = tx.send(JobMsg::Log(format!("job failed: {e}")));
                }
                Err(_) => {
                    let _ = tx.send(JobMsg::Error("job panicked; it was contained".into()));
                }
            }
            let _ = tx.send(JobMsg::Finished);
            done_t.store(true, Ordering::Relaxed);
        })
        .map_err(|e| crate::error::MloError::Other(format!("could not spawn job thread: {e}")))?;

    Ok(JobHandle { id, kind, scope_label, rx, cancel, done })
}

fn run_job(req: JobRequest, cfg: &Config, tx: &Sender<JobMsg>, cancel: &AtomicBool) -> Result<()> {
    let db = Db::open(&cfg.index_db())?;
    let emit = |p: Progress| {
        let _ = tx.send(JobMsg::Progress(p));
    };
    match req {
        JobRequest::Scan(scope) => {
            let model = crate::scan::scan(cfg, &db, &scope, cancel, &emit)?;
            let _ = tx.send(JobMsg::ScanDone(Box::new(model)));
        }
        JobRequest::Script { id, scope } => {
            let progress = |p: Progress| emit(p);
            let ctx = crate::scripts::ScriptCtx { cfg, db: &db, cancel, progress: &progress };
            let outcome = crate::scripts::run(id, &scope, &ctx)?;
            let _ = tx.send(JobMsg::Log(outcome.summary()));
            for r in &outcome.results {
                if !r.note.is_empty() {
                    let _ = tx.send(JobMsg::Log(format!(
                        "{} {}: {}",
                        crate::model::file_name(&r.path),
                        r.outcome.label(),
                        r.note
                    )));
                }
            }
            for n in &outcome.notes {
                let _ = tx.send(JobMsg::Log(n.clone()));
            }
            let _ = tx.send(JobMsg::ScriptDone(Box::new(outcome)));
        }
        JobRequest::LayoutApply { ids, dry_run } => {
            let walked = crate::scan::walk(&cfg.music_folder)?;
            let findings = crate::layout::analyze(&walked, &cfg.music_folder, &Default::default(), cfg);
            emit(Progress { label: "applying layout fixes".into(), current: 0, total: None, note: None });
            let outcomes = crate::layout::apply(cfg, &findings, ids.as_deref(), dry_run)?;
            for o in &outcomes {
                let _ = tx.send(JobMsg::Log(format!(
                    "{} {}: {}",
                    if o.ok { "ok" } else { "fail" },
                    o.action,
                    o.detail
                )));
            }
            let _ = tx.send(JobMsg::LayoutDone(Box::new(outcomes)));
        }
    }
    Ok(())
}