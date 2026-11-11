//! Interface scale: one multiplier that makes everything an application draws
//! larger or smaller, as the operating system's display scaling does.
//!
//! GPUI lays out in logical pixels and draws them at the display's scale
//! factor. [`application`] opens the platform through a wrapper that
//! multiplies that factor by the interface scale. Every size, gap, radius,
//! icon and glyph grows together and stays sharp, because it is drawn at the
//! larger factor rather than enlarged afterwards. The window keeps its size
//! on screen; the application sees fewer logical pixels inside it.
//!
//! ```no_run
//! use gpui_kit::*;
//!
//! struct Hello;
//!
//! impl Render for Hello {
//!     fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
//!         div().child("Hello, World!")
//!     }
//! }
//!
//! fn main() {
//!     gpui_kit::scale::application().run(|cx| {
//!         gpui_kit::init(cx);
//!         gpui_kit::scale::set_scale(1.25, cx);
//!         gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Hello))
//!             .expect("failed to open window");
//!     });
//! }
//! ```
//!
//! Positions the platform reports inside a window (the mouse, touches,
//! dropped files) are divided by the scale on their way in, and positions the
//! application gives the platform inside a window (the window menu, the input
//! method's caret, input regions, client-side decoration insets) are
//! multiplied on their way out. Screen geometry is not scaled: a window's
//! bounds, its display's bounds and [`Window::resize`] stay in the operating
//! system's logical pixels, so saved window sizes keep their meaning when the
//! scale changes.
//!
//! Two things are not scaled yet, because GPUI gives no hook for them: the
//! rectangles an input method asks the input handler for (macOS and Windows
//! place their candidate window with these, Wayland uses the caret position,
//! which is scaled), and the bounds in the accessibility tree.

use std::cell::Cell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use futures::channel::oneshot;
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use smallvec::SmallVec;

use crate::*;

/// The smallest and largest interface scale [`set_scale`] takes.
pub const MIN_SCALE: f32 = 0.5;
pub const MAX_SCALE: f32 = 3.0;

thread_local! {
    /// The interface scale every scaled window draws at. GPUI's platform and
    /// windows live on the main thread, so one per thread is one per app.
    static SCALE: Cell<f32> = const { Cell::new(1.0) };
}

/// The interface scale windows draw at: 1 is the display's own scale.
pub fn scale() -> f32 {
    SCALE.with(Cell::get)
}

/// Draws every scaled window at `scale` times the display's scale, within
/// [`MIN_SCALE`] and [`MAX_SCALE`], and lays them out again. Windows opened
/// through a platform that [`application`] or [`scaled`] did not wrap keep
/// the display's scale.
pub fn set_scale(scale: f32, cx: &mut App) {
    let scale = if scale.is_finite() {
        scale.clamp(MIN_SCALE, MAX_SCALE)
    } else {
        1.0
    };
    if SCALE.with(|cell| cell.replace(scale)) == scale {
        return;
    }
    for window in cx.windows() {
        window
            .update(cx, |_, window, cx| window.bounds_changed(cx))
            .ok();
    }
}

/// The platform's application, with windows drawn at the interface scale.
/// Use it in place of `gpui_kit::application`.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub fn application() -> Application {
    Application::with_platform(scaled(crate::platform::current_platform(false)))
}

/// `platform`, with the windows it opens drawn at the interface scale.
pub fn scaled(platform: Rc<dyn Platform>) -> Rc<dyn Platform> {
    Rc::new(ScaledPlatform(platform))
}

fn inward(position: Point<Pixels>) -> Point<Pixels> {
    position.map(|c| c / scale())
}

fn outward(position: Point<Pixels>) -> Point<Pixels> {
    position.map(|c| c * scale())
}

fn bounds_inward(bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    Bounds::new(inward(bounds.origin), bounds.size.map(|c| c / scale()))
}

fn bounds_outward(bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    Bounds::new(outward(bounds.origin), bounds.size.map(|c| c * scale()))
}

/// `input`, with its positions in the window's logical pixels moved into
/// the scaled ones the application lays out in.
fn input_inward(mut input: PlatformInput) -> PlatformInput {
    match &mut input {
        PlatformInput::MouseDown(event) => event.position = inward(event.position),
        PlatformInput::MouseUp(event) => event.position = inward(event.position),
        PlatformInput::MousePressure(event) => event.position = inward(event.position),
        PlatformInput::MouseMove(event) => event.position = inward(event.position),
        PlatformInput::MouseExited(event) => event.position = inward(event.position),
        PlatformInput::Scroll(event) => {
            event.position = inward(event.position);
            if let ScrollDelta::Pixels(delta) = &mut event.delta {
                *delta = inward(*delta);
            }
        }
        PlatformInput::Pinch(event) => event.position = inward(event.position),
        PlatformInput::LongPress(event) => {
            event.start_position = inward(event.start_position);
            event.position = inward(event.position);
        }
        PlatformInput::TouchDrag(event) => {
            event.start_position = inward(event.start_position);
            event.position = inward(event.position);
        }
        PlatformInput::BackGesture(event) => event.start_position = inward(event.start_position),
        PlatformInput::FileDrop(
            FileDropEvent::Entered { position, .. }
            | FileDropEvent::Pending { position }
            | FileDropEvent::Submit { position },
        ) => *position = inward(*position),
        PlatformInput::Touch(event) => {
            event.position = inward(event.position);
            event.predicted_position = event.predicted_position.map(inward);
        }
        PlatformInput::FileDrop(FileDropEvent::Exited | FileDropEvent::Ended)
        | PlatformInput::KeyDown(_)
        | PlatformInput::KeyUp(_)
        | PlatformInput::ModifiersChanged(_) => {}
    }
    input
}

fn insets_inward(insets: WindowInsets) -> WindowInsets {
    let edges = |edges: Edges<Pixels>| edges.map(|c| *c / scale());
    let mut insets = insets;
    insets.safe_area = edges(insets.safe_area);
    insets.ime = edges(insets.ime);
    insets.corner_radii = insets.corner_radii.map(|c| *c / scale());
    insets
}

struct ScaledPlatform(Rc<dyn Platform>);

impl Platform for ScaledPlatform {
    fn platform_info(&self) -> PlatformInfo {
        self.0.platform_info()
    }
    fn background_executor(&self) -> BackgroundExecutor {
        self.0.background_executor()
    }
    fn foreground_executor(&self) -> ForegroundExecutor {
        self.0.foreground_executor()
    }
    fn text_system(&self) -> Arc<dyn PlatformTextSystem> {
        self.0.text_system()
    }
    fn run(&self, on_finish_launching: Box<dyn 'static + FnOnce()>) {
        self.0.run(on_finish_launching)
    }
    fn quit(&self) {
        self.0.quit()
    }
    fn set_initial_windowing(&self, request: WindowingRequest) {
        self.0.set_initial_windowing(request)
    }
    fn request_windowing(&self, request: WindowingRequest) -> Task<anyhow::Result<()>> {
        self.0.request_windowing(request)
    }
    fn restart(&self, binary_path: Option<PathBuf>, arguments: Vec<OsString>) {
        self.0.restart(binary_path, arguments)
    }
    fn activate(&self, ignoring_other_apps: bool) {
        self.0.activate(ignoring_other_apps)
    }
    fn set_activation_policy(&self, policy: ActivationPolicy) {
        self.0.set_activation_policy(policy)
    }
    fn hide(&self) {
        self.0.hide()
    }
    fn hide_other_apps(&self) {
        self.0.hide_other_apps()
    }
    fn unhide_other_apps(&self) {
        self.0.unhide_other_apps()
    }
    fn displays(&self) -> Vec<Rc<dyn PlatformDisplay>> {
        self.0.displays()
    }
    fn primary_display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        self.0.primary_display()
    }
    fn on_displays_changed(&self, callback: Box<dyn FnMut()>) {
        self.0.on_displays_changed(callback)
    }
    fn active_window(&self) -> Option<AnyWindowHandle> {
        self.0.active_window()
    }
    fn window_stack(&self) -> Option<Vec<AnyWindowHandle>> {
        self.0.window_stack()
    }
    fn is_screen_capture_supported(&self) -> bool {
        self.0.is_screen_capture_supported()
    }
    fn screen_capture_sources(
        &self,
    ) -> oneshot::Receiver<anyhow::Result<Vec<Rc<dyn ScreenCaptureSource>>>> {
        self.0.screen_capture_sources()
    }
    fn open_window(
        &self,
        handle: AnyWindowHandle,
        options: WindowParams,
    ) -> anyhow::Result<Box<dyn PlatformWindow>> {
        Ok(Box::new(ScaledWindow(self.0.open_window(handle, options)?)))
    }
    fn window_appearance(&self) -> WindowAppearance {
        self.0.window_appearance()
    }
    fn set_window_appearance(&self, appearance: Option<WindowAppearance>) {
        self.0.set_window_appearance(appearance)
    }
    fn button_layout(&self) -> Option<WindowButtonLayout> {
        self.0.button_layout()
    }
    fn open_url(&self, url: &str) {
        self.0.open_url(url)
    }
    fn on_open_urls(&self, callback: Box<dyn FnMut(Vec<String>)>) {
        self.0.on_open_urls(callback)
    }
    fn register_url_scheme(&self, url: &str) -> Task<anyhow::Result<()>> {
        self.0.register_url_scheme(url)
    }
    fn prompt_for_paths(
        &self,
        options: PathPromptOptions,
    ) -> oneshot::Receiver<anyhow::Result<Option<Vec<PathBuf>>>> {
        self.0.prompt_for_paths(options)
    }
    fn prompt_for_new_path(
        &self,
        directory: &Path,
        suggested_name: Option<&str>,
    ) -> oneshot::Receiver<anyhow::Result<Option<PathBuf>>> {
        self.0.prompt_for_new_path(directory, suggested_name)
    }
    fn can_select_mixed_files_and_dirs(&self) -> bool {
        self.0.can_select_mixed_files_and_dirs()
    }
    fn reveal_path(&self, path: &Path) {
        self.0.reveal_path(path)
    }
    fn open_with_system(&self, path: &Path) {
        self.0.open_with_system(path)
    }
    fn on_quit(&self, callback: Box<dyn FnMut() -> bool>) {
        self.0.on_quit(callback)
    }
    fn on_reopen(&self, callback: Box<dyn FnMut()>) {
        self.0.on_reopen(callback)
    }
    fn on_system_sleep(&self, callback: Box<dyn FnMut()>) {
        self.0.on_system_sleep(callback)
    }
    fn on_system_wake(&self, callback: Box<dyn FnMut()>) {
        self.0.on_system_wake(callback)
    }
    fn on_app_lifecycle(&self, callback: Box<dyn FnMut(AppLifecyclePhase)>) {
        self.0.on_app_lifecycle(callback)
    }
    fn on_memory_warning(&self, callback: Box<dyn FnMut()>) {
        self.0.on_memory_warning(callback)
    }
    fn gestures(&self) -> Option<Rc<dyn PlatformGestures>> {
        self.0.gestures()
    }
    fn set_menus(&self, menus: Vec<Menu>, keymap: &Keymap) {
        self.0.set_menus(menus, keymap)
    }
    fn get_menus(&self) -> Option<Vec<OwnedMenu>> {
        self.0.get_menus()
    }
    fn set_dock_menu(&self, menu: Vec<MenuItem>, keymap: &Keymap) {
        self.0.set_dock_menu(menu, keymap)
    }
    fn perform_dock_menu_action(&self, action: usize) {
        self.0.perform_dock_menu_action(action)
    }
    fn add_recent_document(&self, path: &Path) {
        self.0.add_recent_document(path)
    }
    fn update_jump_list(
        &self,
        menus: Vec<MenuItem>,
        entries: Vec<SmallVec<[PathBuf; 2]>>,
    ) -> Task<Vec<SmallVec<[PathBuf; 2]>>> {
        self.0.update_jump_list(menus, entries)
    }
    fn on_app_menu_action(&self, callback: Box<dyn FnMut(&dyn Action)>) {
        self.0.on_app_menu_action(callback)
    }
    fn on_will_open_app_menu(&self, callback: Box<dyn FnMut()>) {
        self.0.on_will_open_app_menu(callback)
    }
    fn on_validate_app_menu_command(&self, callback: Box<dyn FnMut(&dyn Action) -> bool>) {
        self.0.on_validate_app_menu_command(callback)
    }
    fn thermal_state(&self) -> ThermalState {
        self.0.thermal_state()
    }
    fn on_thermal_state_change(&self, callback: Box<dyn FnMut()>) {
        self.0.on_thermal_state_change(callback)
    }
    fn prevent_idle_sleep(&self, reason: &str) -> Task<anyhow::Result<ActivityGuard>> {
        self.0.prevent_idle_sleep(reason)
    }
    fn power_mode(&self) -> PowerMode {
        self.0.power_mode()
    }
    fn on_power_mode_change(&self, callback: Box<dyn FnMut()>) {
        self.0.on_power_mode_change(callback)
    }
    fn set_app_identity(&self, identifier: &str, name: &str) {
        self.0.set_app_identity(identifier, name)
    }
    fn show_system_notification(&self, notification: SystemNotification) {
        self.0.show_system_notification(notification)
    }
    fn dismiss_system_notification(&self, tag: &str) {
        self.0.dismiss_system_notification(tag)
    }
    fn on_system_notification_response(
        &self,
        callback: Box<dyn FnMut(SystemNotificationResponse)>,
    ) {
        self.0.on_system_notification_response(callback)
    }
    fn compositor_name(&self) -> &'static str {
        self.0.compositor_name()
    }
    fn graphical_environment(&self) -> Option<GraphicalEnvironment> {
        self.0.graphical_environment()
    }
    fn app_path(&self) -> anyhow::Result<PathBuf> {
        self.0.app_path()
    }
    fn path_for_auxiliary_executable(&self, name: &str) -> anyhow::Result<PathBuf> {
        self.0.path_for_auxiliary_executable(name)
    }
    fn set_cursor_style(&self, style: CursorStyle) {
        self.0.set_cursor_style(style)
    }
    fn hide_cursor_until_mouse_moves(&self) {
        self.0.hide_cursor_until_mouse_moves()
    }
    fn is_cursor_visible(&self) -> bool {
        self.0.is_cursor_visible()
    }
    fn should_auto_hide_scrollbars(&self) -> bool {
        self.0.should_auto_hide_scrollbars()
    }
    fn read_from_clipboard(&self) -> Option<ClipboardItem> {
        self.0.read_from_clipboard()
    }
    fn write_to_clipboard(&self, item: ClipboardItem) {
        self.0.write_to_clipboard(item)
    }
    fn read_from_clipboard_async(&self) -> Task<Result<Option<ClipboardItem>, ClipboardReadError>> {
        self.0.read_from_clipboard_async()
    }
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn read_from_primary(&self) -> Option<ClipboardItem> {
        self.0.read_from_primary()
    }
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn write_to_primary(&self, item: ClipboardItem) {
        self.0.write_to_primary(item)
    }
    #[cfg(target_os = "macos")]
    fn read_from_find_pasteboard(&self) -> Option<ClipboardItem> {
        self.0.read_from_find_pasteboard()
    }
    #[cfg(target_os = "macos")]
    fn write_to_find_pasteboard(&self, item: ClipboardItem) {
        self.0.write_to_find_pasteboard(item)
    }
    fn write_credentials(
        &self,
        url: &str,
        username: &str,
        password: &[u8],
    ) -> Task<anyhow::Result<()>> {
        self.0.write_credentials(url, username, password)
    }
    #[cfg(target_os = "macos")]
    fn write_credentials_if_not_exists(
        &self,
        url: &str,
        username: &str,
        password: &[u8],
    ) -> Task<anyhow::Result<bool>> {
        self.0
            .write_credentials_if_not_exists(url, username, password)
    }
    fn read_credentials(&self, url: &str) -> Task<anyhow::Result<Option<(String, Vec<u8>)>>> {
        self.0.read_credentials(url)
    }
    fn delete_credentials(&self, url: &str) -> Task<anyhow::Result<()>> {
        self.0.delete_credentials(url)
    }
    fn keyboard_layout(&self) -> Box<dyn PlatformKeyboardLayout> {
        self.0.keyboard_layout()
    }
    fn keyboard_mapper(&self) -> Rc<dyn PlatformKeyboardMapper> {
        self.0.keyboard_mapper()
    }
    fn on_keyboard_layout_change(&self, callback: Box<dyn FnMut()>) {
        self.0.on_keyboard_layout_change(callback)
    }
}

struct ScaledWindow(Box<dyn PlatformWindow>);

impl HasWindowHandle for ScaledWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.0.window_handle()
    }
}

impl HasDisplayHandle for ScaledWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.0.display_handle()
    }
}

impl PlatformWindow for ScaledWindow {
    // What the scale changes.

    fn scale_factor(&self) -> f32 {
        self.0.scale_factor() * scale()
    }
    fn content_size(&self) -> Size<Pixels> {
        self.0.content_size().map(|c| c / scale())
    }
    fn resize(&mut self, size: Size<Pixels>) {
        self.0.resize(size.map(|c| c * scale()))
    }
    fn visual_viewport_bounds(&self) -> Bounds<Pixels> {
        bounds_inward(self.0.visual_viewport_bounds())
    }
    fn mouse_position(&self) -> Point<Pixels> {
        inward(self.0.mouse_position())
    }
    fn on_input(&self, mut callback: Box<dyn FnMut(PlatformInput) -> DispatchEventResult>) {
        self.0
            .on_input(Box::new(move |input| callback(input_inward(input))))
    }
    fn on_resize(&self, mut callback: Box<dyn FnMut(Size<Pixels>, f32)>) {
        self.0.on_resize(Box::new(move |size, scale_factor| {
            callback(size.map(|c| c / scale()), scale_factor * scale())
        }))
    }
    #[cfg(target_os = "macos")]
    fn set_traffic_light_position(&self, position: Point<Pixels>) {
        self.0.set_traffic_light_position(outward(position))
    }
    fn show_window_menu(&self, position: Point<Pixels>) {
        self.0.show_window_menu(outward(position))
    }
    fn set_exclusive_zone(&self, zone: Pixels) {
        self.0.set_exclusive_zone(zone * scale())
    }
    fn set_input_region(&self, region: Option<&[Bounds<Pixels>]>) {
        let region: Option<Vec<_>> =
            region.map(|r| r.iter().copied().map(bounds_outward).collect());
        self.0.set_input_region(region.as_deref())
    }
    fn set_client_inset(&self, inset: Pixels) {
        self.0.set_client_inset(inset * scale())
    }
    fn set_client_corner_radii(&self, radii: Corners<Pixels>) {
        self.0.set_client_corner_radii(radii.map(|c| *c * scale()))
    }
    fn update_ime_position(&self, bounds: Bounds<Pixels>) {
        self.0.update_ime_position(bounds_outward(bounds))
    }
    fn insets(&self) -> WindowInsets {
        insets_inward(self.0.insets())
    }
    fn on_insets_changed(&self, mut callback: Box<dyn FnMut(WindowInsets)>) {
        self.0
            .on_insets_changed(Box::new(move |insets| callback(insets_inward(insets))))
    }

    // The rest, as the platform has it.

    fn bounds(&self) -> Bounds<Pixels> {
        self.0.bounds()
    }
    fn is_maximized(&self) -> bool {
        self.0.is_maximized()
    }
    fn window_bounds(&self) -> WindowBounds {
        self.0.window_bounds()
    }
    fn on_visual_viewport_changed(&self, callback: Box<dyn FnMut()>) {
        self.0.on_visual_viewport_changed(callback)
    }
    fn prepare_frame(&self) -> bool {
        self.0.prepare_frame()
    }
    fn appearance(&self) -> WindowAppearance {
        self.0.appearance()
    }
    fn display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        self.0.display()
    }
    fn modifiers(&self) -> Modifiers {
        self.0.modifiers()
    }
    fn capslock(&self) -> Capslock {
        self.0.capslock()
    }
    fn set_input_handler(&mut self, input_handler: PlatformInputHandler) {
        self.0.set_input_handler(input_handler)
    }
    fn take_input_handler(&mut self) -> Option<PlatformInputHandler> {
        self.0.take_input_handler()
    }
    fn set_text_input_configuration(&mut self, configuration: TextInputConfiguration) {
        self.0.set_text_input_configuration(configuration)
    }
    fn prompt(
        &self,
        level: PromptLevel,
        msg: &str,
        detail: Option<&str>,
        answers: &[PromptButton],
    ) -> Option<oneshot::Receiver<usize>> {
        self.0.prompt(level, msg, detail, answers)
    }
    fn activate(&self) {
        self.0.activate()
    }
    fn request_attention(&self) {
        self.0.request_attention()
    }
    fn is_active(&self) -> bool {
        self.0.is_active()
    }
    fn visibility(&self) -> WindowVisibility {
        self.0.visibility()
    }
    fn is_hovered(&self) -> bool {
        self.0.is_hovered()
    }
    fn background_appearance(&self) -> WindowBackgroundAppearance {
        self.0.background_appearance()
    }
    fn set_title(&mut self, title: &str) {
        self.0.set_title(title)
    }
    fn set_background_appearance(&self, background_appearance: WindowBackgroundAppearance) {
        self.0.set_background_appearance(background_appearance)
    }
    fn minimize(&self) {
        self.0.minimize()
    }
    fn zoom(&self) {
        self.0.zoom()
    }
    fn toggle_fullscreen(&self) {
        self.0.toggle_fullscreen()
    }
    fn is_fullscreen(&self) -> bool {
        self.0.is_fullscreen()
    }
    fn frame_waker(&self) -> Option<Rc<dyn Fn()>> {
        self.0.frame_waker()
    }
    fn on_request_frame(&self, callback: Box<dyn FnMut(RequestFrameOptions)>) {
        self.0.on_request_frame(callback)
    }
    fn on_active_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.0.on_active_status_change(callback)
    }
    fn on_visibility_change(&self, callback: Box<dyn FnMut(WindowVisibility)>) {
        self.0.on_visibility_change(callback)
    }
    fn on_hover_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.0.on_hover_status_change(callback)
    }
    fn on_moved(&self, callback: Box<dyn FnMut()>) {
        self.0.on_moved(callback)
    }
    fn on_display_changed(&self, callback: Box<dyn FnMut()>) {
        self.0.on_display_changed(callback)
    }
    fn on_should_close(&self, callback: Box<dyn FnMut() -> bool>) {
        self.0.on_should_close(callback)
    }
    fn on_hit_test_window_control(&self, callback: Box<dyn FnMut() -> Option<WindowControlArea>>) {
        self.0.on_hit_test_window_control(callback)
    }
    fn on_close(&self, callback: Box<dyn FnOnce()>) {
        self.0.on_close(callback)
    }
    fn on_appearance_changed(&self, callback: Box<dyn FnMut()>) {
        self.0.on_appearance_changed(callback)
    }
    fn on_button_layout_changed(&self, callback: Box<dyn FnMut()>) {
        self.0.on_button_layout_changed(callback)
    }
    fn draw(&self, scene: &Scene) {
        self.0.draw(scene)
    }
    #[cfg(feature = "gpui-fast")]
    fn draw_composed(&self, scene: ComposedScene<'_>) {
        self.0.draw_composed(scene)
    }
    #[cfg(feature = "gpui-fast")]
    fn enable_window_composition(&self) -> anyhow::Result<()> {
        self.0.enable_window_composition()
    }
    #[cfg(feature = "gpui-fast")]
    fn create_native_surface(&self) -> anyhow::Result<Rc<dyn PlatformSurfaceAttachment>> {
        self.0.create_native_surface()
    }
    #[cfg(feature = "gpui-fast")]
    fn set_composition_order(
        &self,
        fast_surfaces: &[PlatformCompositionSurface],
    ) -> anyhow::Result<()> {
        self.0.set_composition_order(fast_surfaces)
    }
    fn schedule_frame(&self) {
        self.0.schedule_frame()
    }
    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.0.sprite_atlas()
    }
    fn is_subpixel_rendering_supported(&self) -> bool {
        self.0.is_subpixel_rendering_supported()
    }
    fn get_title(&self) -> String {
        self.0.get_title()
    }
    fn tabbed_windows(&self) -> Option<Vec<SystemWindowTab>> {
        self.0.tabbed_windows()
    }
    fn tab_bar_visible(&self) -> bool {
        self.0.tab_bar_visible()
    }
    fn set_edited(&mut self, edited: bool) {
        self.0.set_edited(edited)
    }
    fn set_document_path(&self, path: Option<&Path>) {
        self.0.set_document_path(path)
    }
    fn toggle_simple_fullscreen(&self) {
        self.0.toggle_simple_fullscreen()
    }
    fn is_simple_fullscreen(&self) -> bool {
        self.0.is_simple_fullscreen()
    }
    fn show_character_palette(&self) {
        self.0.show_character_palette()
    }
    fn titlebar_double_click(&self, is_resizable: bool, is_minimizable: bool) {
        self.0.titlebar_double_click(is_resizable, is_minimizable)
    }
    fn on_move_tab_to_new_window(&self, callback: Box<dyn FnMut()>) {
        self.0.on_move_tab_to_new_window(callback)
    }
    fn on_merge_all_windows(&self, callback: Box<dyn FnMut()>) {
        self.0.on_merge_all_windows(callback)
    }
    fn on_select_previous_tab(&self, callback: Box<dyn FnMut()>) {
        self.0.on_select_previous_tab(callback)
    }
    fn on_select_next_tab(&self, callback: Box<dyn FnMut()>) {
        self.0.on_select_next_tab(callback)
    }
    fn on_toggle_tab_bar(&self, callback: Box<dyn FnMut()>) {
        self.0.on_toggle_tab_bar(callback)
    }
    fn merge_all_windows(&self) {
        self.0.merge_all_windows()
    }
    fn move_tab_to_new_window(&self) {
        self.0.move_tab_to_new_window()
    }
    fn toggle_window_tab_overview(&self) {
        self.0.toggle_window_tab_overview()
    }
    fn set_tabbing_identifier(&self, identifier: Option<String>) {
        self.0.set_tabbing_identifier(identifier)
    }
    fn native_window_state(&self) -> Option<Vec<u8>> {
        self.0.native_window_state()
    }
    fn restore_native_window_state(&self, state: &[u8]) {
        self.0.restore_native_window_state(state)
    }
    #[cfg(target_os = "windows")]
    fn get_raw_handle(&self) -> windows::Win32::Foundation::HWND {
        self.0.get_raw_handle()
    }
    fn inner_window_bounds(&self) -> WindowBounds {
        self.0.inner_window_bounds()
    }
    fn request_decorations(&self, decorations: WindowDecorations) {
        self.0.request_decorations(decorations)
    }
    fn start_window_move(&self) {
        self.0.start_window_move()
    }
    fn can_start_external_drag(&self) -> bool {
        self.0.can_start_external_drag()
    }
    fn start_external_drag(&self, payload: &ExternalDragPayload) -> bool {
        self.0.start_external_drag(payload)
    }
    fn start_window_resize(&self, edge: ResizeEdge) {
        self.0.start_window_resize(edge)
    }
    fn window_decorations(&self) -> Decorations {
        self.0.window_decorations()
    }
    fn set_app_id(&mut self, app_id: &str) {
        self.0.set_app_id(app_id)
    }
    fn map_window(&mut self) -> anyhow::Result<()> {
        self.0.map_window()
    }
    fn window_controls(&self) -> WindowControls {
        self.0.window_controls()
    }
    fn gpu_specs(&self) -> Option<GpuSpecs> {
        self.0.gpu_specs()
    }
    fn set_back_handler(&self, callback: Box<dyn FnMut()>) {
        self.0.set_back_handler(callback)
    }
    fn set_back_enabled(&self, enabled: bool) {
        self.0.set_back_enabled(enabled)
    }
    fn show_soft_keyboard(&self) {
        self.0.show_soft_keyboard()
    }
    fn hide_soft_keyboard(&self) {
        self.0.hide_soft_keyboard()
    }
    fn text_input_state_changed(&self, change: TextInputStateChange) {
        self.0.text_input_state_changed(change)
    }
    fn play_system_bell(&self) {
        self.0.play_system_bell()
    }
    fn a11y_init(&self, callbacks: A11yCallbacks) {
        self.0.a11y_init(callbacks)
    }
    fn a11y_tree_update(&self, tree_update: accesskit::TreeUpdate) {
        self.0.a11y_tree_update(tree_update)
    }
    fn a11y_update_window_bounds(&self) {
        self.0.a11y_update_window_bounds()
    }
}

#[cfg(test)]
mod tests {
    // The glob below brings GPUI's test macro; these are plain tests.
    use core::prelude::v1::test;

    use super::*;

    struct Scaled(f32);

    impl Scaled {
        fn at(scale: f32) -> Self {
            Self(SCALE.with(|cell| cell.replace(scale)))
        }
    }

    impl Drop for Scaled {
        fn drop(&mut self) {
            SCALE.with(|cell| cell.set(self.0));
        }
    }

    #[test]
    fn test_input_positions_are_divided_by_the_scale() {
        let _scale = Scaled::at(2.);
        let input = input_inward(PlatformInput::MouseDown(MouseDownEvent {
            position: point(px(100.), px(40.)),
            ..Default::default()
        }));
        let PlatformInput::MouseDown(event) = input else {
            panic!("a mouse down stays one");
        };
        assert_eq!(event.position, point(px(50.), px(20.)));

        let input = input_inward(PlatformInput::Scroll(ScrollEvent {
            position: point(px(10.), px(10.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-30.))),
            ..Default::default()
        }));
        let PlatformInput::Scroll(event) = input else {
            panic!("a scroll stays one");
        };
        assert_eq!(event.position, point(px(5.), px(5.)));
        assert!(
            matches!(event.delta, ScrollDelta::Pixels(delta) if delta == point(px(0.), px(-15.)))
        );
    }

    #[test]
    fn test_line_scrolls_keep_their_lines() {
        let _scale = Scaled::at(1.5);
        let input = input_inward(PlatformInput::Scroll(ScrollEvent {
            delta: ScrollDelta::Lines(point(0., 3.)),
            ..Default::default()
        }));
        let PlatformInput::Scroll(event) = input else {
            panic!("a scroll stays one");
        };
        assert!(matches!(event.delta, ScrollDelta::Lines(lines) if lines == point(0., 3.)));
    }

    #[test]
    fn test_positions_round_trip_through_the_scale() {
        let _scale = Scaled::at(1.25);
        let bounds = Bounds::new(point(px(8.), px(16.)), size(px(40.), px(20.)));
        assert_eq!(bounds_inward(bounds_outward(bounds)), bounds);
        assert_eq!(outward(point(px(8.), px(16.))), point(px(10.), px(20.)));
    }
}
