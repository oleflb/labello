use std::{cell::Cell, error::Error, future::Future, pin::Pin, rc::Rc, time::Duration};

type LocalTask = Pin<Box<dyn Future<Output = ()> + 'static>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InspectorMode {
    Demo,
    Live,
}

fn main() -> Result<(), Box<dyn Error>> {
    let mode = parse_mode(std::env::args().skip(1))
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    eframe::run_native(
        "Labello MCP Inspector",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1440.0, 1000.0]),
            ..Default::default()
        },
        Box::new(move |creation_context| {
            creation_context.egui_ctx.enable_accesskit();
            Ok(Box::new(InspectorApp::new(mode)?))
        }),
    )?;
    Ok(())
}

fn parse_mode(args: impl IntoIterator<Item = String>) -> Result<InspectorMode, String> {
    let args = args.into_iter().collect::<Vec<_>>();
    match args.as_slice() {
        [] => Ok(InspectorMode::Demo),
        [argument] if argument == "--live" => Ok(InspectorMode::Live),
        _ => Err("Usage: labello-egui-mcp-inspector [--live]".to_string()),
    }
}

struct InspectorApp {
    app: labello_ui::LabelloApp,
    executor: Option<Rc<NativeExecutor>>,
}

impl InspectorApp {
    fn new(mode: InspectorMode) -> std::io::Result<Self> {
        match mode {
            InspectorMode::Demo => Ok(Self {
                app: labello_ui::LabelloApp::default(),
                executor: None,
            }),
            InspectorMode::Live => {
                let executor = Rc::new(NativeExecutor::new()?);
                let executor_for_spawner = executor.clone();
                let mut config = labello_ui::AppConfig::default();
                config.dev_token.clear();
                config.user_id = "admin".into();
                let mut app = labello_ui::LabelloApp::live_http(config);
                app.set_native_task_spawner(move |task| executor_for_spawner.spawn(task));
                Ok(Self {
                    app,
                    executor: Some(executor),
                })
            }
        }
    }
}

impl eframe::App for InspectorApp {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, frame: &mut eframe::Frame) {
        if let Some(executor) = &self.executor {
            executor.tick();
        }
        eframe::App::ui(&mut self.app, ui, frame);
        if self
            .executor
            .as_ref()
            .is_some_and(|executor| executor.pending_tasks() > 0)
        {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
    }
}

struct NativeExecutor {
    runtime: tokio::runtime::Runtime,
    tasks: Rc<tokio::task::LocalSet>,
    pending: Rc<Cell<usize>>,
}

impl NativeExecutor {
    fn new() -> std::io::Result<Self> {
        Ok(Self {
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
            tasks: Rc::new(tokio::task::LocalSet::new()),
            pending: Rc::new(Cell::new(0)),
        })
    }

    fn spawn(&self, task: LocalTask) {
        self.pending.set(self.pending.get() + 1);
        let pending = self.pending.clone();
        self.tasks.spawn_local(async move {
            let _pending_task = PendingTask(pending);
            task.await;
        });
    }

    fn tick(&self) {
        if self.pending_tasks() == 0 {
            return;
        }
        self.runtime.block_on(
            self.tasks
                .run_until(async { tokio::task::yield_now().await }),
        );
    }

    fn pending_tasks(&self) -> usize {
        self.pending.get()
    }
}

struct PendingTask(Rc<Cell<usize>>);

impl Drop for PendingTask {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use super::*;

    #[test]
    fn demo_mode_is_the_default_and_live_mode_is_explicit() {
        assert_eq!(
            parse_mode(Vec::<String>::new()).unwrap(),
            InspectorMode::Demo
        );
        assert_eq!(
            parse_mode(["--live".to_string()]).unwrap(),
            InspectorMode::Live
        );
    }

    #[test]
    fn unknown_arguments_are_rejected() {
        let error = parse_mode(["--unknown".to_string()]).unwrap_err();
        assert!(error.contains("Usage:"));
    }

    #[test]
    fn native_executor_advances_and_finishes_local_tasks() {
        let executor = NativeExecutor::new().unwrap();
        let completed = Rc::new(Cell::new(false));
        let completed_for_task = completed.clone();
        executor.spawn(Box::pin(async move {
            completed_for_task.set(true);
        }));

        assert_eq!(executor.pending_tasks(), 1);
        executor.tick();

        assert!(completed.get());
        assert_eq!(executor.pending_tasks(), 0);
    }

    #[test]
    fn native_executor_drives_timer_tasks() {
        let executor = NativeExecutor::new().unwrap();
        let completed = Rc::new(Cell::new(false));
        let completed_for_task = completed.clone();
        executor.spawn(Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(1)).await;
            completed_for_task.set(true);
        }));

        for _ in 0..50 {
            executor.tick();
            if completed.get() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }

        assert!(completed.get());
        assert_eq!(executor.pending_tasks(), 0);
    }
}
