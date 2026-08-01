use std::num::NonZeroU32;
use std::sync::Arc;

use apksule_apk::ApkPackage;
use apksule_compat::{Context as AndroidContext, ResourceSource};
use softbuffer::{Context as SoftContext, Surface};
use thiserror::Error;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle};
use winit::window::{Window, WindowId};

use crate::dex::{DexError, DexRuntime, InterpretingDexRuntime};
use crate::input::InputTranslator;
use crate::lifecycle::{ActivityLifecycle, ActivityState, LifecycleError};
use crate::renderer::{RenderError, render_launch_surface, render_view_surface};

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("compatibility context could not be created: {0}")]
    Compat(#[from] apksule_compat::CompatError),
    #[error("DEX boundary failed: {0}")]
    Dex(#[from] DexError),
    #[error("Activity lifecycle failed: {0}")]
    Lifecycle(#[from] LifecycleError),
    #[error("window event loop failed: {0}")]
    EventLoop(String),
    #[error("window could not be created: {0}")]
    Window(String),
    #[error("software surface failed: {0}")]
    Surface(String),
    #[error("launch surface rendering failed: {0}")]
    Render(#[from] RenderError),
    #[error("runtime terminated after a fatal window error: {0}")]
    Fatal(String),
}

/// Runtime entry point used by the launcher. Host concerns stop at this boundary.
#[derive(Debug, Default, Clone, Copy)]
pub struct Runtime;

impl Runtime {
    pub fn launch(package: ApkPackage) -> Result<(), RuntimeError> {
        let event_loop =
            EventLoop::new().map_err(|error| RuntimeError::EventLoop(error.to_string()))?;
        event_loop.set_control_flow(ControlFlow::Wait);

        let soft_context = SoftContext::new(event_loop.owned_display_handle())
            .map_err(|error| RuntimeError::Surface(error.to_string()))?;

        let source = Arc::new(ApkResourceSource { package: package.clone() });
        let gms_signals = gms_signals(&package);
        let android_context =
            AndroidContext::new(package.package_name.clone(), source, &gms_signals)?;
        tracing::info!(
            package = %package.package_name,
            main_activity = ?package.main_activity,
            storage = %android_context.storage().root().display(),
            gms = ?android_context.gms().detection().availability,
            "runtime context created"
        );

        let mut vm: Box<dyn DexRuntime> = Box::new(InterpretingDexRuntime::new());
        vm.load(&package, &android_context)?;
        vm.on_lifecycle(ActivityState::Created)?;

        let mut application = RuntimeApplication::new(package, android_context, vm, soft_context);
        event_loop
            .run_app(&mut application)
            .map_err(|error| RuntimeError::EventLoop(error.to_string()))?;
        if let Some(error) = application.fatal_error {
            return Err(RuntimeError::Fatal(error));
        }
        Ok(())
    }
}

struct RuntimeApplication {
    package: ApkPackage,
    _android_context: AndroidContext,
    vm: Box<dyn DexRuntime>,
    lifecycle: ActivityLifecycle,
    input: InputTranslator,
    soft_context: SoftContext<OwnedDisplayHandle>,
    window: Option<Arc<Window>>,
    surface: Option<Surface<OwnedDisplayHandle, Arc<Window>>>,
    fatal_error: Option<String>,
}

impl RuntimeApplication {
    fn new(
        package: ApkPackage,
        android_context: AndroidContext,
        vm: Box<dyn DexRuntime>,
        soft_context: SoftContext<OwnedDisplayHandle>,
    ) -> Self {
        Self {
            package,
            _android_context: android_context,
            vm,
            lifecycle: ActivityLifecycle::new(),
            input: InputTranslator::default(),
            soft_context,
            window: None,
            surface: None,
            fatal_error: None,
        }
    }

    fn create_or_restore_surface(
        &mut self,
        event_loop: &ActiveEventLoop,
    ) -> Result<(), RuntimeError> {
        if self.window.is_none() {
            let title =
                self.package.application_label.as_deref().unwrap_or(&self.package.package_name);
            let attributes = Window::default_attributes()
                .with_title(title)
                .with_inner_size(LogicalSize::new(960.0, 640.0))
                .with_min_inner_size(LogicalSize::new(520.0, 360.0));
            let window = Arc::new(
                event_loop
                    .create_window(attributes)
                    .map_err(|error| RuntimeError::Window(error.to_string()))?,
            );
            self.window = Some(window);
        }

        if self.surface.is_none() {
            let window = self.window.as_ref().expect("window was initialized").clone();
            self.surface = Some(
                Surface::new(&self.soft_context, window)
                    .map_err(|error| RuntimeError::Surface(error.to_string()))?,
            );
        }

        self.resume_lifecycle()?;
        if let Some(window) = &self.window {
            let size = window.inner_size();
            self.vm.on_surface_changed(size.width, size.height)?;
            window.request_redraw();
        }
        Ok(())
    }

    fn resume_lifecycle(&mut self) -> Result<(), RuntimeError> {
        match self.lifecycle.state() {
            ActivityState::Created | ActivityState::Stopped => {
                self.deliver_state(ActivityState::Started)?;
                self.deliver_state(ActivityState::Resumed)?;
            }
            ActivityState::Started | ActivityState::Paused => {
                self.deliver_state(ActivityState::Resumed)?;
            }
            ActivityState::Resumed => {}
            ActivityState::Destroyed => {
                return Err(RuntimeError::Fatal(
                    "received resume after Activity destruction".to_owned(),
                ));
            }
        }
        Ok(())
    }

    fn suspend_lifecycle(&mut self) -> Result<(), RuntimeError> {
        if self.lifecycle.state() == ActivityState::Resumed {
            self.deliver_state(ActivityState::Paused)?;
        }
        if matches!(self.lifecycle.state(), ActivityState::Paused | ActivityState::Started) {
            self.deliver_state(ActivityState::Stopped)?;
        }
        Ok(())
    }

    fn destroy_lifecycle(&mut self) -> Result<(), RuntimeError> {
        self.suspend_lifecycle()?;
        if self.lifecycle.state() == ActivityState::Created
            || self.lifecycle.state() == ActivityState::Stopped
        {
            self.deliver_state(ActivityState::Destroyed)?;
        }
        Ok(())
    }

    fn deliver_state(&mut self, state: ActivityState) -> Result<(), RuntimeError> {
        self.lifecycle.transition(state)?;
        self.vm.on_lifecycle(state)?;
        tracing::info!(?state, "Activity lifecycle transition");
        Ok(())
    }

    fn render(&mut self) -> Result<(), RuntimeError> {
        let Some(window) = self.window.as_ref() else {
            return Ok(());
        };
        let size = window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return Ok(());
        };
        let frame = match self.vm.ui_host() {
            Some(host) if host.has_content() => {
                render_view_surface(host, size.width, size.height)?
            }
            _ => render_launch_surface(&self.package, self.vm.status(), size.width, size.height)?,
        };
        let surface = self.surface.as_mut().ok_or_else(|| {
            RuntimeError::Surface("redraw requested without an active surface".to_owned())
        })?;
        surface.resize(width, height).map_err(|error| RuntimeError::Surface(error.to_string()))?;
        let mut buffer =
            surface.buffer_mut().map_err(|error| RuntimeError::Surface(error.to_string()))?;
        buffer.copy_from_slice(&frame);
        buffer.present().map_err(|error| RuntimeError::Surface(error.to_string()))
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: impl std::fmt::Display) {
        let error = error.to_string();
        tracing::error!(%error, "runtime window failed");
        self.fatal_error.get_or_insert(error);
        event_loop.exit();
    }
}

impl ApplicationHandler for RuntimeApplication {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.create_or_restore_surface(event_loop) {
            self.fail(event_loop, error);
        }
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.suspend_lifecycle() {
            self.fail(event_loop, error);
        }
        self.surface = None;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        if window.id() != window_id {
            return;
        }

        if let Some(input) = self.input.translate(&event)
            && let Err(error) = self.vm.on_input(&input)
        {
            self.fail(event_loop, error);
            return;
        }

        let result = match event {
            WindowEvent::CloseRequested => {
                let result = self.destroy_lifecycle();
                event_loop.exit();
                result
            }
            WindowEvent::Resized(size) => {
                let result = self.vm.on_surface_changed(size.width, size.height);
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
                result.map_err(RuntimeError::from)
            }
            WindowEvent::RedrawRequested => self.render(),
            _ => Ok(()),
        };
        if let Err(error) = result {
            self.fail(event_loop, error);
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if self.lifecycle.state() != ActivityState::Destroyed
            && let Err(error) = self.destroy_lifecycle()
        {
            tracing::error!(%error, "failed to finish Activity lifecycle");
            self.fatal_error.get_or_insert_with(|| error.to_string());
        }
    }
}

#[derive(Debug, Clone)]
struct ApkResourceSource {
    package: ApkPackage,
}

impl ResourceSource for ApkResourceSource {
    fn contains(&self, path: &str) -> bool {
        self.package.contains_entry(path)
    }

    fn load(&self, path: &str) -> std::result::Result<Vec<u8>, String> {
        self.package.read_entry(path).map_err(|error| error.to_string())
    }
}

fn gms_signals(package: &ApkPackage) -> Vec<String> {
    package
        .permissions
        .iter()
        .cloned()
        .chain(package.activities.iter().map(|activity| activity.name.clone()))
        .chain(package.components.iter().map(|component| component.name.clone()))
        .chain(package.entries.iter().map(|entry| entry.path.clone()))
        .collect()
}
