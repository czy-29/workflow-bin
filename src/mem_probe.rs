pub struct MemProbe {
    handle: std::thread::JoinHandle<(u64, u64)>,
    signal: std::sync::mpsc::Sender<()>,
}

impl MemProbe {
    pub fn new() -> Self {
        let pid = sysinfo::get_current_pid().unwrap();
        let mut sys = sysinfo::System::new();
        let (send, recv) = std::sync::mpsc::channel();

        Self {
            handle: std::thread::spawn(move || {
                let mut max = 0;
                let mut sample = 0u64;

                loop {
                    sample += 1;
                    sys.refresh_processes_specifics(
                        sysinfo::ProcessesToUpdate::Some(&[pid]),
                        sysinfo::ProcessRefreshKind::new().with_memory(),
                    );

                    let this = sys.process(pid).unwrap().memory();

                    if this > max {
                        max = this;
                    }

                    if recv.try_recv().is_ok() {
                        return (max, sample);
                    }

                    std::thread::sleep(std::time::Duration::from_nanos(1));
                }
            }),
            signal: send,
        }
    }

    pub fn join_and_get_mb_sample(self) -> (f64, u64) {
        self.signal.send(()).unwrap();
        let (bytes, sample) = self.handle.join().unwrap();
        (bytes as f64 / 1024.0 / 1024.0, sample)
    }
}
