use std::env;

#[cfg(windows)]
mod service {
    use std::ffi::OsString;
    use std::time::Duration;
    use windows_service::{
        define_windows_service,
        service::{
            ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
            ServiceType,
        },
        service_control_handler::{self, ServiceControlHandlerResult},
        service_dispatcher,
    };

    pub const SERVICE_NAME: &str = "DockPanelHostHelper";
    pub const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

    pub fn start_service() -> Result<(), windows_service::Error> {
        service_dispatcher::start(SERVICE_NAME, ffi_service_main)?;
        Ok(())
    }

    define_windows_service!(ffi_service_main, my_service_main);

    fn my_service_main(_arguments: Vec<OsString>) {
        let _ = run_service_loop();
    }

    fn run_service_loop() -> Result<(), windows_service::Error> {
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);

        let event_handler = move |control_event| -> ServiceControlHandlerResult {
            match control_event {
                ServiceControl::Stop | ServiceControl::Shutdown => {
                    let _ = shutdown_tx.blocking_send(());
                    ServiceControlHandlerResult::NoError
                }
                ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
                _ => ServiceControlHandlerResult::NotImplemented,
            }
        };

        let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;

        let running_status = ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: ServiceState::Running,
            controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            exit_code: ServiceExitCode::NO_ERROR,
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        };
        status_handle.set_service_status(running_status)?;

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| windows_service::Error::Winapi(e))?;

        rt.block_on(async {
            tokio::select! {
                res = dockpanel_host_helper::server::run_named_pipe_server(None) => {
                    if let Err(e) = res {
                        eprintln!("Pipe server exited with error: {e}");
                    }
                }
                _ = shutdown_rx.recv() => {
                    println!("Service received shutdown signal.");
                }
            }
        });

        status_handle.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: ServiceState::Stopped,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::NO_ERROR,
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })?;

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let standalone = args.iter().any(|arg| arg == "--run" || arg == "-r" || arg == "--standalone");

    if standalone {
        println!("DockPanel Host Helper running in standalone console mode...");
        dockpanel_host_helper::server::run_named_pipe_server(None).await?;
        return Ok(());
    }

    #[cfg(windows)]
    {
        // Attempt to connect to Windows Service Control Manager (SCM).
        // If not launched by SCM (e.g. executed manually in console without --run),
        // fallback to standalone console mode.
        println!("Attempting Windows Service startup...");
        if let Err(err) = service::start_service() {
            println!("Service dispatcher failed ({err}); falling back to standalone console mode...");
            dockpanel_host_helper::server::run_named_pipe_server(None).await?;
        }
    }

    #[cfg(not(windows))]
    {
        println!("Non-Windows environment detected. Running standalone...");
        dockpanel_host_helper::server::run_named_pipe_server(None).await?;
    }

    Ok(())
}
