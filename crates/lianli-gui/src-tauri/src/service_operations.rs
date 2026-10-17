use crate::command_error::CommandError;
use lianli_control::ui_text::text;
use lianli_shared::installation::InstallationContext;
use lianli_shared::services::{ServiceActionRequest, ServiceChangeRequest, ServiceOperationStatus};
use lianli_shared::ui_message::UiMessage;
use lianli_shared::ui_message::UiText;
use std::sync::{Mutex, OnceLock};
use tauri::{Emitter, Manager};

static STATUS: OnceLock<Mutex<ServiceOperationStatus>> = OnceLock::new();
static SUBMITTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn state() -> &'static Mutex<ServiceOperationStatus> {
    STATUS.get_or_init(|| {
        Mutex::new(ServiceOperationStatus {
            active: false,
            message: String::new(),
            message_ui: None,
            success: None,
        })
    })
}

pub fn status() -> ServiceOperationStatus {
    if !SUBMITTING.load(std::sync::atomic::Ordering::Acquire) {
        let observed = match observed_status() {
            Ok(Some(status)) => Some(status),
            Ok(None) => state()
                .lock()
                .unwrap()
                .active
                .then(|| failed(progress_disappeared())),
            Err(error) => {
                let error = format!("{error:#}");
                Some(failed(UiText::new(
                    UiMessage::new("gui.service.progressUnreadable").param("error", &error),
                    format!("Cannot read service progress: {error}"),
                )))
            }
        };
        if let Some(observed) = observed {
            let mut current = state().lock().unwrap();
            if !SUBMITTING.load(std::sync::atomic::Ordering::Acquire) {
                *current = observed;
            }
        }
    }
    state().lock().unwrap().clone()
}

fn observed_status() -> anyhow::Result<Option<ServiceOperationStatus>> {
    let action = lianli_control::operation_job::read()?;
    let change = if InstallationContext::detect() != InstallationContext::UnsupportedContainer {
        lianli_control::switch_job::read()?
    } else {
        None
    };
    Ok(match (action, change) {
        (Some(action), Some(change)) => {
            if change.status.active
                || (!action.status.active && change.updated_at_ms >= action.updated_at_ms)
            {
                Some(change.status)
            } else {
                Some(action.status)
            }
        }
        (Some(action), None) => Some(action.status),
        (None, Some(change)) => Some(change.status),
        (None, None) => None,
    })
}

pub fn active() -> bool {
    status().active
}

pub struct Operation {
    app: tauri::AppHandle,
    finished: bool,
}

impl Operation {
    pub fn begin(app: tauri::AppHandle) -> Result<Self, CommandError> {
        if app.get_webview_window("editor").is_some() {
            return Err(CommandError::ui(
                "Save and close the template editor before managing services.",
                UiMessage::new("gui.service.closeEditor"),
            ));
        }
        let busy = || {
            CommandError::ui(
                "Another service action is in progress.",
                UiMessage::new("gui.service.busy"),
            )
        };
        if status().active {
            return Err(busy());
        }
        {
            let mut status = state().lock().unwrap();
            if status.active {
                return Err(busy());
            }
            *status = ServiceOperationStatus {
                active: true,
                message: "Checking service state…".into(),
                message_ui: Some(UiMessage::new("control.serviceAction.checkingState")),
                success: None,
            };
            SUBMITTING.store(true, std::sync::atomic::Ordering::Release);
        }
        let operation = Self {
            app,
            finished: false,
        };
        operation.emit();
        Ok(operation)
    }

    fn emit(&self) {
        if let Err(error) = self
            .app
            .emit("service-operation", state().lock().unwrap().clone())
        {
            tracing::warn!("Cannot send service-operation progress: {error}");
        }
    }

    pub fn run_package_update(self) -> Result<(), CommandError> {
        self.submit(|| {
            lianli_control::app_update::launch().map_err(|error| {
                let error = format!("{error:#}");
                CommandError::ui(
                    &error,
                    UiMessage::new("gui.appUpdate.failed").param("error", &error),
                )
            })?;
            Ok(ServiceOperationStatus {
                active: false,
                success: Some(true),
                message: "Update terminal is ready.".into(),
                message_ui: Some(UiMessage::new("gui.appUpdate.ready")),
            })
        })
    }

    pub fn run(self, request: ServiceActionRequest) -> Result<(), CommandError> {
        self.submit(|| {
            let id = lianli_control::operation_job::start(&InstallationContext::detect(), request)
                .map_err(CommandError::from)?;
            let record = lianli_control::operation_job::read()
                .map_err(|error| {
                    observe_failed(
                        "gui.service.observeFailed",
                        "Cannot observe service progress",
                        error,
                    )
                })?
                .ok_or_else(|| text_error(progress_disappeared()))?;
            if record.id != id {
                return Err(CommandError::ui(
                    "Another operation replaced this progress record. Recheck actual services.",
                    UiMessage::new("gui.service.replaced"),
                ));
            }
            Ok(record.status)
        })
    }

    pub fn run_change(self, request: ServiceChangeRequest) -> Result<(), CommandError> {
        self.submit(|| {
            let id = lianli_control::switch_job::start(request).map_err(CommandError::from)?;
            let record = lianli_control::switch_job::read()
                .map_err(|error| {
                    observe_failed(
                        "gui.service.observeSwitchFailed",
                        "Cannot observe switch progress",
                        error,
                    )
                })?
                .ok_or_else(|| {
                    CommandError::ui(
                        "Switch progress disappeared. Recheck before retrying.",
                        UiMessage::new("gui.service.switchProgressDisappeared"),
                    )
                })?;
            if record.id != id {
                return Err(CommandError::ui(
                    "Another switch replaced the progress record. Recheck actual services.",
                    UiMessage::new("gui.service.switchReplaced"),
                ));
            }
            Ok(record.status)
        })
    }

    pub fn run_setup(
        self,
        deployment: lianli_control::container_deployment::Deployment,
    ) -> Result<(), CommandError> {
        self.submit(|| {
            lianli_control::container_bootstrap::install(&deployment)
                .map_err(CommandError::from)?;
            let installed = text(
                "gui.service.hostSupportInstalled",
                "Host support installed. Choose which service mode to start.",
            );
            Ok(ServiceOperationStatus {
                active: false,
                success: Some(true),
                message: installed.english,
                message_ui: installed.ui,
            })
        })
    }

    fn submit(
        mut self,
        submit: impl FnOnce() -> Result<ServiceOperationStatus, CommandError>,
    ) -> Result<(), CommandError> {
        let result = submit().map(|status| *state().lock().unwrap() = status);
        if let Err(error) = &result {
            let mut status = state().lock().unwrap();
            status.active = false;
            status.success = Some(false);
            status.message = error.message.clone();
            status.message_ui = error.ui.clone();
        }
        SUBMITTING.store(false, std::sync::atomic::Ordering::Release);
        self.finished = true;
        self.emit();
        result
    }
}

impl Drop for Operation {
    fn drop(&mut self) {
        if !self.finished {
            SUBMITTING.store(false, std::sync::atomic::Ordering::Release);
            *state().lock().unwrap() = failed(text(
                "gui.service.submissionInterrupted",
                "GUI submission was interrupted. Recheck independent service progress before retrying.",
            ));
            self.emit();
        }
    }
}

fn failed(text: UiText) -> ServiceOperationStatus {
    ServiceOperationStatus {
        active: false,
        success: Some(false),
        message: text.english,
        message_ui: text.ui,
    }
}

fn progress_disappeared() -> UiText {
    text(
        "gui.service.progressDisappeared",
        "Service progress disappeared. Recheck actual services before retrying.",
    )
}

fn text_error(text: UiText) -> CommandError {
    CommandError {
        message: text.english,
        ui: text.ui,
    }
}

fn observe_failed(key: &str, english: &str, error: anyhow::Error) -> CommandError {
    let error = format!("{error:#}");
    CommandError::ui(
        format!("{english}: {error}"),
        UiMessage::new(key).param("error", &error),
    )
}
