//! Bounded metadata-only profiling shared by resource generations.
use serde::Serialize;
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize)]
pub struct ProfileSpan {
    pub id: u64,
    pub parent: Option<u64>,
    pub resource: String,
    pub generation: u64,
    pub language: String,
    pub phase: String,
    pub source: Option<String>,
    pub start_us: u64,
    pub wall_time_us: u64,
    pub host_cpu_time_us: Option<u64>,
    pub exclusive_host_cpu_time_us: Option<u64>,
    /// Total CPU used by all worker-process threads during this IPC exchange.
    pub worker_cpu_time_us: Option<u64>,
    /// Time already spent in the local host event queue before dispatch.
    pub queue_wait_us: Option<u64>,
    /// Time blocked receiving IPC; includes worker execution, not pure queueing.
    pub ipc_receive_wait_us: Option<u64>,
    pub failed: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct ProfileSummary {
    pub resource: String,
    pub generation: u64,
    pub phase: String,
    pub samples: usize,
    pub p50_wall_time_us: u64,
    pub p95_wall_time_us: u64,
    pub p99_wall_time_us: u64,
    pub max_wall_time_us: u64,
    pub exclusive_host_cpu_time_us: Option<u64>,
    pub worker_cpu_time_us: Option<u64>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ProfileSnapshot {
    pub origin_unix_us: u64,
    pub enabled: bool,
    pub capacity: usize,
    pub retention_ms: u64,
    pub evicted: u64,
    pub timing_contract: &'static str,
    pub spans: Vec<ProfileSpan>,
    pub summaries: Vec<ProfileSummary>,
}
#[derive(Debug)]
pub(crate) struct History {
    origin: Instant,
    origin_unix_us: u64,
    enabled: bool,
    capacity: usize,
    retention: Duration,
    evicted: u64,
    spans: VecDeque<ProfileSpan>,
}
pub(crate) type Recorder = Arc<Mutex<History>>;
#[derive(Clone, Debug)]
pub(crate) struct Context {
    pub recorder: Recorder,
    pub resource: String,
    pub generation: u64,
    pub language: String,
    pub source: Option<String>,
}
impl History {
    pub(crate) fn new() -> Recorder {
        Arc::new(Mutex::new(Self {
            origin: Instant::now(),
            origin_unix_us: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_micros()
                .min(u64::MAX as u128) as u64,
            enabled: true,
            capacity: 4096,
            retention: Duration::from_secs(60),
            evicted: 0,
            spans: VecDeque::new(),
        }))
    }
    pub(crate) fn configure(
        &mut self,
        enabled: bool,
        capacity: usize,
        retention_ms: u64,
    ) -> Result<(), String> {
        if !(64..=8192).contains(&capacity) || !(100..=300_000).contains(&retention_ms) {
            return Err("profile bounds: 64..8192 spans and 100..300000 ms".into());
        }
        self.enabled = enabled;
        self.capacity = capacity;
        self.retention = Duration::from_millis(retention_ms);
        self.prune();
        Ok(())
    }
    fn now(&self) -> u64 {
        self.origin.elapsed().as_micros().min(u64::MAX as u128) as u64
    }
    fn prune(&mut self) {
        let cutoff = self.now().saturating_sub(self.retention.as_micros() as u64);
        // Nested spans finish before their parents, so completion insertion order
        // does not imply chronological start order. Retention uses completion.
        while self
            .spans
            .front()
            .is_some_and(|s| s.start_us.saturating_add(s.wall_time_us) < cutoff)
        {
            self.spans.pop_front();
            self.evicted = self.evicted.saturating_add(1);
        }
        while self.spans.len() > self.capacity {
            self.spans.pop_front();
            self.evicted = self.evicted.saturating_add(1);
        }
    }
    pub(crate) fn snapshot(&mut self) -> ProfileSnapshot {
        self.prune();
        let mut groups: BTreeMap<(String, u64, String), Vec<&ProfileSpan>> = BTreeMap::new();
        for span in &self.spans {
            groups
                .entry((span.resource.clone(), span.generation, span.phase.clone()))
                .or_default()
                .push(span);
        }
        let mut summaries: Vec<_> = groups
            .into_iter()
            .map(|((resource, generation, phase), spans)| {
                let mut wall: Vec<_> = spans.iter().map(|s| s.wall_time_us).collect();
                wall.sort_unstable();
                let percentile = |p: usize| wall[(wall.len() * p).div_ceil(100).saturating_sub(1)];
                let sum = |f: fn(&ProfileSpan) -> Option<u64>| {
                    let values: Vec<_> = spans.iter().filter_map(|s| f(s)).collect();
                    (!values.is_empty()).then(|| values.into_iter().fold(0u64, u64::saturating_add))
                };
                ProfileSummary {
                    resource,
                    generation,
                    phase,
                    samples: spans.len(),
                    p50_wall_time_us: percentile(50),
                    p95_wall_time_us: percentile(95),
                    p99_wall_time_us: percentile(99),
                    max_wall_time_us: *wall.last().unwrap(),
                    exclusive_host_cpu_time_us: sum(|s| s.exclusive_host_cpu_time_us),
                    worker_cpu_time_us: sum(|s| s.worker_cpu_time_us),
                }
            })
            .collect();
        summaries.sort_by_key(|s| std::cmp::Reverse(s.p95_wall_time_us));
        ProfileSnapshot {
            origin_unix_us: self.origin_unix_us,
            enabled: self.enabled,
            capacity: self.capacity,
            retention_ms: self.retention.as_millis() as u64,
            evicted: self.evicted,
            timing_contract: "Wall and host CPU are inclusive; only exclusive_host_cpu_time_us is additive on the host thread. Worker CPU is measured separately and only on IPC spans. IPC receive wait includes worker execution and is not queue waiting. Queue wait precedes dispatch. Histograms describe retained samples, not lifetime totals.",
            spans: self.spans.iter().cloned().collect(),
            summaries,
        }
    }
}
impl ProfileSnapshot {
    pub fn chrome_trace(&self) -> serde_json::Value {
        let events:Vec<_>=self.spans.iter().map(|s|serde_json::json!({"name":s.phase,"cat":s.language,"ph":"X","ts":s.start_us,"dur":s.wall_time_us,"pid":1,"tid":1,
          "args":{"resource":s.resource,"generation":s.generation.to_string(),"span":s.id.to_string(),"parent":s.parent.map(|id|id.to_string()),"source":s.source,"host_cpu_us":s.host_cpu_time_us,"exclusive_host_cpu_us":s.exclusive_host_cpu_time_us,"worker_cpu_us":s.worker_cpu_time_us,"queue_wait_us":s.queue_wait_us,"ipc_receive_wait_us":s.ipc_receive_wait_us,"failed":s.failed}})).collect();
        serde_json::json!({"traceEvents":events,"displayTimeUnit":"ms","metadata":{"origin_unix_us":self.origin_unix_us,"contract":self.timing_contract,"capacity":self.capacity,"retention_ms":self.retention_ms,"evicted":self.evicted}})
    }
}
struct Active {
    id: u64,
    recorder: usize,
    children_cpu: u64,
}
thread_local! {static ACTIVE:RefCell<Vec<Active>>=const {RefCell::new(Vec::new())};}
static NEXT: AtomicU64 = AtomicU64::new(1);
pub(crate) struct SpanTimer {
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
    context: Context,
    id: u64,
    parent: Option<u64>,
    start_us: u64,
    phase: String,
    source: Option<String>,
    closed: bool,
}
fn label(value: &str) -> String {
    value.chars().take(160).collect()
}
impl SpanTimer {
    pub(crate) fn start(context: Context, phase: &str, source: Option<String>) -> Option<Self> {
        let history = context.recorder.lock().unwrap();
        if !history.enabled {
            return None;
        }
        let start_us = history.now();
        drop(history);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let recorder = Arc::as_ptr(&context.recorder) as usize;
        let parent = ACTIVE.with(|active| {
            let mut active = active.borrow_mut();
            let parent = active
                .last()
                .filter(|a| a.recorder == recorder)
                .map(|a| a.id);
            active.push(Active {
                id,
                recorder,
                children_cpu: 0,
            });
            parent
        });
        let source = source.or_else(|| context.source.clone()).map(|s| label(&s));
        Some(Self {
            _thread: std::marker::PhantomData,
            context,
            id,
            parent,
            start_us,
            phase: label(phase),
            source,
            closed: false,
        })
    }
    pub(crate) fn finish(
        mut self,
        wall: u64,
        cpu: Option<u64>,
        worker: Option<u64>,
        queue: Option<u64>,
        ipc: Option<u64>,
        failed: bool,
    ) {
        let children = ACTIVE
            .with(|active| {
                let mut active = active.borrow_mut();
                let index = active.iter().rposition(|a| a.id == self.id)?;
                let entry = active.remove(index);
                if let Some(parent) = active.last_mut().filter(|a| Some(a.id) == self.parent) {
                    parent.children_cpu = parent.children_cpu.saturating_add(cpu.unwrap_or(0));
                }
                Some(entry.children_cpu)
            })
            .unwrap_or(0);
        self.closed = true;
        let mut history = self.context.recorder.lock().unwrap();
        history.spans.push_back(ProfileSpan {
            id: self.id,
            parent: self.parent,
            resource: self.context.resource.clone(),
            generation: self.context.generation,
            language: self.context.language.clone(),
            phase: self.phase.clone(),
            source: self.source.clone(),
            start_us: self.start_us,
            wall_time_us: wall,
            host_cpu_time_us: cpu,
            exclusive_host_cpu_time_us: cpu.map(|v| v.saturating_sub(children)),
            worker_cpu_time_us: worker,
            queue_wait_us: queue,
            ipc_receive_wait_us: ipc,
            failed,
        });
        history.prune();
    }
}
impl Drop for SpanTimer {
    fn drop(&mut self) {
        if !self.closed {
            ACTIVE.with(|active| active.borrow_mut().retain(|a| a.id != self.id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_expires_and_rejects_unbounded_configuration() {
        let recorder = History::new();
        assert!(recorder.lock().unwrap().configure(true, 8193, 100).is_err());
        assert!(
            recorder
                .lock()
                .unwrap()
                .configure(true, 64, 300001)
                .is_err()
        );
        recorder.lock().unwrap().configure(true, 64, 100).unwrap();
        let context = Context {
            recorder: recorder.clone(),
            resource: "test".into(),
            generation: 1,
            language: "lua".into(),
            source: None,
        };
        let timer = SpanTimer::start(context, "callback", None).unwrap();
        timer.finish(1, Some(0), None, None, None, false);
        assert_eq!(recorder.lock().unwrap().snapshot().spans.len(), 1);
        std::thread::sleep(Duration::from_millis(110));
        let snapshot = recorder.lock().unwrap().snapshot();
        assert!(snapshot.spans.is_empty());
        assert_eq!(snapshot.evicted, 1);
    }
    #[test]
    fn direct_child_cpu_is_subtracted_once() {
        let recorder = History::new();
        let context = Context {
            recorder: recorder.clone(),
            resource: "test".into(),
            generation: 1,
            language: "lua".into(),
            source: None,
        };
        let outer = SpanTimer::start(context.clone(), "outer", None).unwrap();
        let middle = SpanTimer::start(context.clone(), "middle", None).unwrap();
        let inner = SpanTimer::start(context, "inner", None).unwrap();
        inner.finish(3, Some(3), None, None, None, false);
        middle.finish(7, Some(7), None, None, None, false);
        outer.finish(11, Some(11), None, None, None, false);
        let snapshot = recorder.lock().unwrap().snapshot();
        assert_eq!(
            snapshot
                .spans
                .iter()
                .map(|s| s.exclusive_host_cpu_time_us.unwrap())
                .sum::<u64>(),
            11
        );
        assert_eq!(snapshot.spans[1].exclusive_host_cpu_time_us, Some(4));
        assert_eq!(snapshot.spans[2].exclusive_host_cpu_time_us, Some(4));
    }
}

/// Owns a profiling span without borrowing the resource host. Dropping closes it.
/// A scope must stay on its originating host thread because CPU counters and
/// parent-span nesting belong to that thread.
///
/// ```compile_fail
/// use skate_mods::resources::ProfileScope;
/// fn requires_send<T: Send>() {}
/// requires_send::<ProfileScope>();
/// ```
pub struct ProfileScope {
    timer: Option<crate::runtime_metrics::Timer>,
}
impl ProfileScope {
    pub(crate) fn new(timer: crate::runtime_metrics::Timer) -> Self {
        Self { timer: Some(timer) }
    }
    pub fn finish(mut self, failed: bool) {
        if let Some(timer) = self.timer.take() {
            timer.finish_profile(failed);
        }
    }
}
impl Drop for ProfileScope {
    fn drop(&mut self) {
        if let Some(timer) = self.timer.take() {
            timer.finish_profile(false);
        }
    }
}
