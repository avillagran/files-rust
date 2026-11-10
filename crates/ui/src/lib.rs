mod app_identity;
mod archive_operation;
mod archive_preview_job;
mod archive_recovery;
mod archive_ui;
mod audio_preview_job;
mod automatic_sizes;
mod aux_io;
mod bookmarks_state;
mod bookmarks_ui;
mod breadcrumb_scene;
mod breadcrumb_ui;
mod breadcrumbs;
mod capture_frame;
mod checksum_job;
mod checksum_ui;
mod child_watch;
#[cfg(all(test, target_os = "linux"))]
mod clipboard_flow_tests;
mod clipboard_scene;
mod clipboard_ui;
mod columns;
mod columns_scene;
mod columns_view;
mod command_palette;
mod context_menu;
mod copy_dialog;
mod copy_job;
mod copy_name_field;
mod copy_names;
mod copy_report;
mod copy_request_builder;
mod copy_review;
mod copy_scene;
mod copy_state;
mod copy_ui;
mod delete_ui;
mod devices_ui;
/// Background transport owned by the visible directory comparison panel.
mod directory_compare_job;
mod directory_compare_ui;
mod directory_sync_job;
mod document_source;
mod du_job;
mod dual_pane;
mod dual_scene;
mod dual_view;
mod duplicate_job;
mod file_clipboard;
mod file_manager_bus;
mod file_manager_preflight;
mod file_manager_ui;
mod filter_edit;
mod folder_summary;
mod folder_views_state;
mod folder_views_ui;
mod fonts;
mod grid_view;
mod help_view;
#[cfg(test)]
mod hidden_context_tests;
mod hidden_scene;
mod html_display_list;
mod html_preview_job;
mod image_preview_job;
mod image_preview_view;
mod key_commands;
mod keyboard_nav;
mod line_edit;
mod line_field;
mod list_columns;
mod listing_sync;
mod listing_watch;
mod map_actions;
mod markdown_document;
mod markdown_preview_job;
mod markdown_render;
mod media_watch;
mod model;
mod motion;
mod mutation_scene;
mod mutation_sheet_state;
mod mutation_sheets;
mod mutations;
mod native_drag_origin;
mod native_drop;
mod native_transfer;
mod native_transfer_io;
#[cfg(test)]
mod navigation_tests;
mod network_ui;
mod notice_navigation;
mod office_preview_job;
mod ofolder_ui;
mod omarchy_theme;
mod omi;
mod omnibar_input;
mod open_job;
mod open_scene;
mod open_with_ui;
mod operation_bar;
mod operation_feedback;
mod operation_notices;
mod owned_child;
pub mod painter;
mod path_edit;
mod pdf_preview_job;
mod places_sidebar;
mod platform_wait;
mod preview_link;
mod preview_scene;
mod properties_dialog;
mod properties_job;
mod properties_scene;
mod properties_ui;
#[cfg(all(test, target_os = "linux"))]
mod properties_worker_tests;
mod readonly_gate;
mod refresh_selection;
mod rename_dialog;
mod rename_job;
mod save_queue;
mod saved_search_state;
mod saved_search_ui;
mod search_marks;
mod search_order;
mod search_page_cache;
mod search_pages_job;
mod search_state;
mod search_stream;
mod search_transfer;
mod search_ui;
mod selection_tools;
mod session_codec;
mod session_state;
mod session_store;
mod session_ui;
mod sftp_executor;
mod sftp_profiles_state;
mod sftp_profiles_ui;
mod sftp_report;
mod sftp_report_ui;
mod sftp_state;
mod sftp_ui;
mod sidebar_motion_scene;
mod sidebar_order;
mod size_panel;
mod size_queue;
mod size_report;
mod sort_controls;
mod space_map_job;
mod space_map_scene;
mod space_map_state;
mod space_map_view;
mod spatial_view;
mod status_tips;
mod tab_context;
mod tab_context_scene;
mod tab_gesture_scene;
mod tab_gestures;
mod tab_motion_scene;
mod tab_strip;
mod tabs;
mod tabs_scene;
mod tabs_ui;
mod text_document;
mod text_paste_gate;
mod text_preview;
mod text_preview_job;
mod text_preview_state;
mod text_preview_view;
mod theme_watch;
mod thumbnail_cache;
mod thumbnail_disk_cache;
mod thumbnail_job;
mod thumbnails;
mod toolbar;
mod toolkit_menu;
mod trash_browser;
mod trash_history;
mod trash_scene;
mod video_preview_job;
mod video_stream_job;
mod view;
mod wheel_scroll;
mod window_chrome;
mod window_presentation;
mod window_scene;
mod worker;
use fltk::{
    app,
    enums::{Damage, Event, FrameType, Key, Shortcut},
    frame::Frame,
    prelude::*,
    window::Window,
};
use model::{Action, Model, View};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

fn input(m: &mut Model, text: &str) {
    m.text_paste.invalidate();
    m.columns.deferred_menu = None;
    m.session.restore_allowed = false;
    if !m.editing {
        if let Err(e) = m.filter_input(text) {
            m.error = e.into();
        }
        return;
    }
    match m.path_input(text) {
        Err(e) => m.error = e.into(),
        Ok(()) => {
            if m.error == omnibar_input::TOO_LONG {
                m.error.clear();
            }
            m.replace_edit = false;
        }
    }
}

fn event(m: &mut Model, ev: Event, w: i32, h: i32) -> bool {
    if matches!(ev, Event::Push | Event::KeyDown | Event::Shortcut) {
        m.cancel_parent_return_focus();
        m.text_paste.invalidate();
        m.wheel_scroll.cancel();
    }
    if m.delete.confirm.is_some() && !m.closing {
        return crate::delete_ui::event(m, ev);
    }
    if m.open_with.is_some() && !m.closing {
        return crate::open_with_ui::event(m, ev);
    }
    if m.trash_browser.is_some() && !m.closing {
        return crate::trash_browser::event(m, ev);
    }
    if m.closing {
        if ev == Event::Push {
            m.pointer_down(app::event_x(), app::event_y(), false, false, false);
        }
        if (ev == Event::KeyDown || ev == Event::Shortcut) && app::event_key() == Key::Escape {
            m.closing = false;
            m.session.closing_saved = false;
        }
        return true;
    }
    if m.comparison.is_some() {
        return crate::directory_compare_ui::event(m, ev);
    }
    if m.checksums.is_some() {
        return crate::checksum_ui::event(m, ev);
    }
    if m.sftp.form.is_some() {
        return crate::sftp_ui::event(m, ev);
    }
    if m.saved_searches.panel.is_some() {
        return crate::saved_search_ui::event(m, ev);
    }
    if m.bookmarks.panel.is_some() {
        return crate::bookmarks_ui::event(m, ev, w, h);
    }
    if m.commands.is_some() {
        return crate::command_palette::event(m, ev);
    }
    if m.search.visible && m.copy_form.is_some() {
        return crate::copy_ui::event(m, ev);
    }
    if m.search.visible && m.notice_open {
        return crate::mutations::event(m, ev);
    }
    if m.search.visible {
        return crate::search_ui::event(m, ev, w, h);
    }
    if m.size_panel.is_some() {
        return crate::size_panel::event(m, ev);
    }
    if matches!(ev, Event::KeyDown | Event::Shortcut)
        && app::event_key() == Key::from_char(' ')
        && app::event_state().intersects(Shortcut::Ctrl | Shortcut::Meta)
        && app::event_state().intersects(Shortcut::Alt)
    {
        m.action(Action::Sizes);
        return true;
    }
    if matches!(ev, Event::KeyDown | Event::Shortcut | Event::Push) {
        m.session.restore_allowed = false;
    }
    if matches!(ev, Event::KeyDown | Event::Shortcut)
        && crate::command_palette::shortcut(m, app::event_key(), app::event_state())
    {
        return true;
    }
    if matches!(ev, Event::KeyDown | Event::Shortcut)
        && (app::event_key() == Key::F1
            || crate::key_commands::help_chord(
                app::event_key(),
                app::event_state(),
                &app::event_text(),
            ))
    {
        m.action(Action::Help);
        return true;
    }
    if matches!(ev, Event::KeyDown | Event::Shortcut)
        && app::event_state().intersects(Shortcut::Ctrl | Shortcut::Meta)
    {
        let command = match app::event_key() {
            k if k == Key::from_char('t') => {
                Some(if app::event_state().intersects(Shortcut::Shift) {
                    tabs::Command::Reopen
                } else {
                    tabs::Command::New
                })
            }
            k if k == Key::from_char('w') => Some(tabs::Command::Close(m.tabs.active)),
            Key::Tab => Some(tabs::Command::Cycle(
                if app::event_state().intersects(Shortcut::Shift) {
                    -1
                } else {
                    1
                },
            )),
            _ => None,
        };
        if let Some(command) = command {
            m.action(Action::Tab(command));
            return true;
        }
    }
    // Ctrl+PgUp/PgDn and Alt+1..9; an open PDF preview keeps Ctrl+PgUp/PgDn.
    if matches!(ev, Event::KeyDown | Event::Shortcut)
        && !(m.preview.visible() && m.preview.pdf_available())
        && let Some(command) =
            crate::key_commands::tab_chord(m, app::event_key(), app::event_state())
    {
        m.action(Action::Tab(command));
        return true;
    }
    if m.help.is_some() {
        match ev {
            Event::Push => m.pointer_down(app::event_x(), app::event_y(), false, false, false),
            Event::KeyDown | Event::Shortcut => help_view::key(m, app::event_key()),
            Event::MouseWheel => m.action(Action::HelpScroll(app::event_dy_value())),
            _ => {}
        }
        return true;
    }
    if m.context_menu.is_some() && !m.toolkit_popup {
        match ev {
            Event::Push => crate::context_menu::pointer(
                m,
                app::event_x(),
                app::event_y(),
                app::event_button() == 1,
            ),
            Event::KeyDown | Event::Shortcut => crate::context_menu::key(m, app::event_key()),
            _ => {}
        }
        return true;
    }
    if m.properties_dialog.is_some() {
        return crate::properties_ui::event(m, ev, h);
    }
    if m.recovery_catalog.open {
        return crate::mutations::event(m, ev);
    }
    if m.notice_open {
        return crate::mutations::event(m, ev);
    }
    if m.copy_form.is_some() {
        return crate::copy_ui::event(m, ev);
    }
    if m.preview.visible() {
        return crate::text_preview::event(m, ev);
    }
    if m.rename_dialog.is_some() || m.notice_open || m.mutation_applied {
        if m.toolkit_mutation_sheet && m.rename_dialog.is_some() {
            return true;
        }
        return crate::mutations::event(m, ev);
    }
    if matches!(ev, Event::KeyDown | Event::Shortcut)
        && crate::key_commands::rail_chord(app::event_key(), app::event_state())
    {
        m.action(Action::FocusRail);
        return true;
    }
    // Single pane: Tab goes to the places rail, Shift+Tab to the tab strip.
    // Dual pane keeps Tab for switching panes (below).
    if ev == Event::KeyDown
        && app::event_key() == Key::Tab
        && !m.dual.enabled
        && !m.filter_edit.active
        && !m.editing
        && m.sidebar_widget
        && !app::event_state().intersects(Shortcut::Ctrl | Shortcut::Meta | Shortcut::Alt)
    {
        m.focus_request = Some(if app::event_state().intersects(Shortcut::Shift) {
            crate::key_commands::FocusTarget::Tabs
        } else {
            crate::key_commands::FocusTarget::Rail
        });
        return true;
    }
    if !m.editing && matches!(ev, Event::KeyDown | Event::Shortcut) && app::event_key() == Key::F9 {
        m.action(Action::DualPane);
        return true;
    }
    if matches!(ev, Event::KeyDown | Event::Shortcut)
        && app::event_key() == Key::Tab
        && m.dual.enabled
        && !m.filter_edit.active
        && !m.editing
        && !app::event_state().intersects(Shortcut::Ctrl | Shortcut::Meta | Shortcut::Alt)
    {
        m.switch_dual();
        return true;
    }
    match ev {
        Event::Focus | Event::Unfocus => true,
        Event::Enter | Event::Move | Event::Leave => {
            let x = app::event_x();
            let y = app::event_y();
            let old = m.rail;
            // The toolkit Sidebar owns hover-expand when it is present.
            if !m.sidebar_widget {
                m.rail = ev != Event::Leave
                    && y >= 90
                    && y < crate::operation_bar::browser_height(m, h) - 32
                    && x < if m.rail { 170 } else { 52 };
            }
            let hover = m.hits.iter().rev().find_map(|hit| {
                if hit.generation == m.generation && hit.rect.contains(x, y) {
                    if let Action::Entry(i) = hit.action {
                        Some(i)
                    } else {
                        None
                    }
                } else {
                    None
                }
            });
            let hovering_tip = ev != Event::Leave && view::tip_rect(w, h).contains(x, y);
            m.status_tips.set_hover(hovering_tip);
            let toolbar_hover = if ev == Event::Leave || m.editing || window_chrome::blocked(m) {
                None
            } else {
                Some((x, y))
            };
            let toolbar_changed = m.toolbar.hover(toolbar_hover);
            let changed = old != m.rail || hover != m.hovered || toolbar_changed;
            m.hovered = hover;
            changed
        }
        Event::MouseWheel => {
            m.cancel_parent_return_focus();
            m.folder_restore = None;
            if m.operation_bar && app::event_y() >= h - 32 - crate::operation_bar::HEIGHT {
                return true;
            }

            if m.dual.enabled {
                crate::wheel_scroll::wheel(m, app::event_x(), app::event_dy_value(), false, h);
                return true;
            }
            if m.view == View::SpaceMap {
                crate::wheel_scroll::wheel(m, app::event_x(), app::event_dy_value(), false, h);
                return true;
            }
            if m.view == View::Columns
                && (app::event_state().intersects(Shortcut::Shift) || app::event_dx_value() != 0)
            {
                let delta = if app::event_dx_value() != 0 {
                    app::event_dx_value()
                } else {
                    app::event_dy_value()
                };
                crate::wheel_scroll::wheel(m, app::event_x(), delta, true, h);
                return true;
            }
            if m.view == View::Columns {
                crate::wheel_scroll::wheel(m, app::event_x(), app::event_dy_value(), false, h);
                return true;
            }
            if app::event_dy_value() == 0 {
                return true;
            }
            if m.view == View::Grid && app::event_state().intersects(Shortcut::Ctrl) {
                m.action(Action::GridZoom(if app::event_dy_value() < 0 {
                    1.1
                } else {
                    1. / 1.1
                }));
                return true;
            }
            if m.view == View::Spatial && app::event_state().intersects(Shortcut::Ctrl) {
                m.action(Action::Zoom(if app::event_dy_value() < 0 {
                    1.1
                } else {
                    1.0 / 1.1
                }));
                return true;
            }
            if m.view == View::Spatial && app::event_state().intersects(Shortcut::Shift) {
                m.pan_x = (m.pan_x + app::event_dy_value() as f32 * 38.0).max(0.0);
                return true;
            }
            m.folder_restore = None;
            crate::wheel_scroll::wheel(m, app::event_x(), app::event_dy_value(), false, h);
            true
        }
        Event::Push => {
            if crate::operation_bar::pointer(m, app::event_x(), app::event_y()) {
                return true;
            }
            if crate::filter_edit::pointer(
                m,
                app::event_x(),
                app::event_y(),
                true,
                false,
                app::event_state().intersects(Shortcut::Shift),
                app::event_clicks(),
            ) {
                return true;
            }
            m.end_filter_edit();
            if crate::path_edit::pointer(
                m,
                app::event_x(),
                app::event_y(),
                true,
                false,
                app::event_state().intersects(Shortcut::Shift),
                app::event_clicks(),
            ) {
                return true;
            }
            if m.editing
                && !m.hits.iter().rev().any(|hit| {
                    hit.rect.contains(app::event_x(), app::event_y())
                        && matches!(hit.action, Action::PathCommit | Action::PathCancel)
                })
            {
                m.end_path_edit();
            }
            m.pointer_down(
                app::event_x(),
                app::event_y(),
                app::event_state().intersects(Shortcut::Ctrl | Shortcut::Meta),
                app::event_state().intersects(Shortcut::Shift),
                app::event_clicks(),
            );
            true
        }
        Event::Drag => {
            if crate::filter_edit::pointer(
                m,
                app::event_x(),
                app::event_y(),
                false,
                true,
                false,
                false,
            ) {
                return true;
            }
            if crate::path_edit::pointer(
                m,
                app::event_x(),
                app::event_y(),
                false,
                true,
                false,
                false,
            ) {
                return true;
            }
            m.pointer_drag(app::event_x(), app::event_y());
            true
        }
        Event::Released => {
            crate::filter_edit::pointer(m, 0, 0, false, false, false, false);
            crate::path_edit::pointer(
                m,
                app::event_x(),
                app::event_y(),
                false,
                false,
                false,
                false,
            );
            m.pointer_up();
            true
        }
        Event::Paste => {
            input(m, &app::event_text());
            true
        }
        Event::KeyDown | Event::Shortcut => {
            let key = app::event_key();
            let ctrl = app::event_state().intersects(Shortcut::Ctrl | Shortcut::Meta);
            if crate::operation_bar::shortcut(
                m,
                key,
                ctrl,
                app::event_state().intersects(Shortcut::Alt),
            ) {
                return true;
            }

            match m.path_key(
                key,
                ctrl,
                app::event_state().intersects(Shortcut::Shift),
                app::event_state().intersects(Shortcut::Alt),
            ) {
                path_edit::KeyResult::Handled => return true,
                path_edit::KeyResult::Copy(text) => {
                    if !text.is_empty() {
                        app::copy(&text);
                    }
                    return true;
                }
                path_edit::KeyResult::Unhandled => {}
            }
            let text = app::event_text();
            let compose = if m.filter_edit.active
                && !ctrl
                && !text.is_empty()
                && text.chars().all(|c| !c.is_control())
            {
                app::compose().unwrap_or(0).max(0) as usize
            } else {
                0
            };
            match m.filter_key(key, app::event_state(), &text, compose) {
                path_edit::KeyResult::Handled => return true,
                path_edit::KeyResult::Copy(text) => {
                    if !text.is_empty() {
                        app::copy(&text);
                    }
                    return true;
                }
                path_edit::KeyResult::Unhandled => {}
            }
            if ctrl && key == Key::from_char('f') {
                m.action(if app::event_state().intersects(Shortcut::Shift) {
                    Action::Search
                } else {
                    Action::Filter
                });
                return true;
            }
            if ctrl && !m.editing && key == Key::from_char(' ') {
                m.action(if app::event_state().intersects(Shortcut::Shift) {
                    Action::CalculateListedSizes
                } else {
                    Action::CalculateSize
                });
                return true;
            }
            if ctrl && key == Key::from_char('d') && !app::event_state().intersects(Shortcut::Shift)
            {
                m.action(if app::event_state().intersects(Shortcut::Alt) {
                    Action::Bookmarks
                } else {
                    Action::BookmarkAdd
                });
                return true;
            }
            if key == Key::Insert && !m.editing && !ctrl {
                m.mark_and_advance();
                wheel_scroll::glide(m, h, |m| view::reveal_selection(m, w, h));
                return true;
            }
            if ctrl
                && !m.editing
                && key == Key::from_char('o')
                && app::event_state().intersects(Shortcut::Shift)
            {
                m.action(Action::OpenWith);
                return true;
            }
            if ctrl
                && !m.editing
                && key == Key::from_char('s')
                && !app::event_state().intersects(Shortcut::Shift)
            {
                m.action(Action::SelectPattern);
                return true;
            }
            if ctrl
                && !m.editing
                && key == Key::from_char('m')
                && !app::event_state().intersects(Shortcut::Shift)
            {
                m.action(Action::LinkCopied);
                return true;
            }
            // Nautilus: Ctrl+. terminal here, Shift+Ctrl+M create link.
            if ctrl && !m.editing && key == Key::from_char('.') {
                m.action(Action::Terminal);
                return true;
            }
            if ctrl
                && !m.editing
                && key == Key::from_char('m')
                && app::event_state().intersects(Shortcut::Shift)
            {
                m.action(Action::CreateLinks);
                return true;
            }
            if ctrl
                && !m.editing
                && key == Key::from_char('c')
                && app::event_state().intersects(Shortcut::Alt)
            {
                m.action(Action::CopyPath);
                return true;
            }
            if key == Key::F3 && !m.editing {
                m.action(Action::Preview);
                return true;
            }
            if key == Key::F8 && !m.editing {
                m.action(Action::Trash);
                return true;
            }
            if key == Key::Down && app::event_state().intersects(Shortcut::Alt) && !m.editing {
                m.action(Action::Breadcrumbs);
                return true;
            }
            if app::event_state().intersects(Shortcut::Alt)
                && (key == Key::Left || key == Key::Right)
            {
                m.action(Action::History(key == Key::Right));
                return true;
            }
            if key == Key::Enter && !m.editing && app::event_state().intersects(Shortcut::Alt) {
                m.action(Action::Properties);
                return true;
            }
            if key == Key::from_char(' ') && !m.editing && !ctrl {
                m.action(Action::Preview);
                return true;
            }
            if ctrl && !m.editing && (key == Key::from_char('c') || key == Key::from_char('x')) {
                m.action(if key == Key::from_char('x') {
                    Action::ClipboardCut
                } else {
                    Action::ClipboardCopy
                });
                return true;
            }
            if ctrl && key == Key::from_char('h') {
                m.action(Action::ToggleHidden);
                return true;
            }
            if ctrl && key == Key::from_char('r') {
                m.action(Action::Refresh);
                return true;
            }
            if key == Key::F10 && ctrl && app::event_state().intersects(Shortcut::Shift) {
                tab_context::keyboard(m);
                return true;
            }
            if ctrl
                && app::event_state().intersects(Shortcut::Shift)
                && matches!(key, Key::PageUp | Key::PageDown)
            {
                if let Some(command) =
                    tab_context::move_command(m, m.tabs.active, key == Key::PageDown)
                {
                    m.action(Action::Tab(command));
                }
                return true;
            }
            if key == Key::Menu || key == Key::F10 && app::event_state().intersects(Shortcut::Shift)
            {
                crate::context_menu::open_keyboard(m, w, h);
                return true;
            }
            if key == Key::Up && app::event_state().contains(Shortcut::Alt) && !m.editing {
                m.action(Action::Parent);
                return true;
            }
            if key == Key::Home && app::event_state().contains(Shortcut::Alt) && !ctrl && !m.editing
            {
                m.action(Action::GoHome);
                return true;
            }
            if ctrl
                && app::event_state().contains(Shortcut::Alt)
                && key == Key::from_char('i')
                && !m.editing
            {
                m.action(Action::Inspector);
                return true;
            }
            if key == Key::F10 {
                m.action(Action::Actions);
                return true;
            }
            if ctrl && key == Key::from_char(',') {
                m.action(Action::Settings);
                return true;
            }
            if key == Key::F6 {
                m.action(Action::MoveFiles);
                return true;
            }
            if key == Key::F5 {
                crate::keyboard_nav::f5(m);
                return true;
            }
            if key == Key::Delete && !m.editing {
                if app::event_state().intersects(Shortcut::Shift) {
                    m.action(Action::DeletePermanently);
                } else {
                    m.action(Action::Trash);
                }
                return true;
            }
            if key == Key::F2 {
                m.action(Action::Rename);
                return true;
            }
            if key == Key::F7
                || ctrl
                    && app::event_state().intersects(Shortcut::Shift)
                    && key == Key::from_char('n')
            {
                m.action(Action::NewEntry(true));
                return true;
            }
            if ctrl && key == Key::from_char('n') {
                m.action(Action::NewEntry(false));
                return true;
            }
            if ctrl && key == Key::from_char('z') {
                m.action(Action::Undo(app::event_state().intersects(Shortcut::Shift)));
                return true;
            }
            if ctrl && key == Key::from_char('l') {
                m.action(Action::Omni);
                return true;
            }
            if ctrl && key == Key::from_char('1') {
                m.action(Action::View(View::Groups));
                return true;
            }
            if ctrl && key == Key::from_char('2') {
                m.action(Action::View(View::List));
                return true;
            }
            if ctrl && key == Key::from_char('5') {
                m.action(Action::View(View::Grid));
                return true;
            }
            if ctrl
                && m.view == View::Grid
                && (key == Key::from_char('+')
                    || key == Key::from_char('=')
                    || key == Key::from_char('-'))
            {
                m.action(Action::GridZoom(if key == Key::from_char('-') {
                    1. / 1.15
                } else {
                    1.15
                }));
                return true;
            }
            if ctrl && key == Key::from_char('0') && matches!(m.view, View::Grid | View::Spatial) {
                m.action(Action::ZoomStep(0));
                return true;
            }
            if ctrl && key == Key::from_char('4') {
                m.action(Action::View(View::SpaceMap));
                return true;
            }
            if ctrl && key == Key::from_char('3') {
                m.action(Action::View(View::Columns));
                return true;
            }
            if ctrl && key == Key::from_char('a') && !m.editing {
                m.action(Action::Selection(
                    if app::event_state().intersects(Shortcut::Shift) {
                        crate::selection_tools::Command::Filtered
                    } else {
                        crate::selection_tools::Command::All
                    },
                ));
                return true;
            }
            if ctrl
                && !m.editing
                && app::event_state().intersects(Shortcut::Shift)
                && key == Key::from_char('d')
            {
                m.action(Action::Selection(crate::selection_tools::Command::Clear));
                return true;
            }
            if ctrl && key == Key::from_char('i') && !m.editing {
                m.action(Action::Selection(crate::selection_tools::Command::Invert));
                return true;
            }
            if ctrl
                && (key == Key::from_char('+') || key == Key::from_char('='))
                && m.view == View::Spatial
            {
                m.action(Action::Zoom(1.15));
                return true;
            }
            if ctrl && key == Key::from_char('-') && m.view == View::Spatial {
                m.action(Action::Zoom(1.0 / 1.15));
                return true;
            }
            if ctrl && key == Key::from_char('a') && m.editing {
                m.replace_edit = true;
                return true;
            }
            if key == Key::Escape {
                if m.settings || m.actions_menu {
                    m.settings = false;
                    m.actions_menu = false;
                } else if m.editing {
                    m.end_path_edit();
                } else {
                    m.escape_browser();
                }
                return true;
            }
            if key == Key::Enter || key == Key::KPEnter {
                let mods = app::event_state();
                if m.editing {
                    m.commit_path_edit();
                } else if mods.intersects(Shortcut::Ctrl | Shortcut::Meta) {
                    m.open_focused_in_tab(true);
                } else if mods.intersects(Shortcut::Shift) {
                    m.open_focused_in_tab(false);
                } else if !m.open_marked() {
                    m.open_selected();
                }
                return true;
            }
            if key == Key::BackSpace {
                if m.editing {
                    m.path_edit
                        .cursor
                        .delete(&mut m.edit, true, false, m.replace_edit);
                    m.replace_edit = false;
                } else {
                    crate::keyboard_nav::backspace(m);
                }
                return true;
            }
            if crate::keyboard_nav::key(
                m,
                key,
                ctrl,
                app::event_state().intersects(Shortcut::Shift),
                w,
                h,
            ) {
                return true;
            }
            if ctrl {
                return false;
            }
            let text = app::event_text();
            if !text.is_empty() && text.chars().all(|c| !c.is_control()) {
                if let Some(delete) = app::compose() {
                    if delete <= 0 && m.seed_location(&text) {
                        return true;
                    }
                    if m.editing {
                        if let Err(error) = m.path_compose(&text, delete.max(0) as usize) {
                            m.error = error.into();
                        }
                        return true;
                    }
                    if let Err(e) = m.filter_compose(&text, delete.max(0) as usize) {
                        m.error = e.into();
                    }
                    return true;
                }
                if !m.seed_location(&text) {
                    input(m, &text);
                }
                return true;
            }
            false
        }
        _ => false,
    }
}

/// One latest-wins loader per window; only its newest request can open a menu.
/// Disk access runs in the toolkit's bounded worker pool.
fn recent_loader(
    win: &Window,
    model: &Rc<RefCell<Model>>,
    canvas: &Frame,
) -> fltk_omarchy::task::Latest<Result<Vec<files_core::recent::Recent>, String>> {
    let (win, model, canvas) = (win.clone(), model.clone(), canvas.clone());
    fltk_omarchy::task::Latest::new(
        Duration::ZERO,
        move |result: Result<Vec<files_core::recent::Recent>, String>| {
            if win.was_deleted() || canvas.was_deleted() || !win.shown() {
                return;
            }
            if model
                .try_borrow()
                .is_ok_and(|m| m.closing || places_sidebar::covered(&m))
            {
                return;
            }
            let notice = |text: &str| {
                if let Ok(mut state) = model.try_borrow_mut() {
                    state.operation_message = Some(text.into());
                }
                canvas.clone().redraw();
            };
            match result {
                Ok(list) if list.is_empty() => notice("No hay archivos recientes"),
                Ok(list) => {
                    eprintln!("FILES_TOOLKIT_RECENT count={}", list.len());
                    let items = places_sidebar::recent_items(&list);
                    let (m, c) = (model.clone(), canvas.clone());
                    fltk_omarchy::overlay::Menu::open(&win, "Recientes", items, move |i| {
                        if let Some(r) = list.get(i)
                            && let Ok(mut state) = m.try_borrow_mut()
                        {
                            let ok = if r.folder {
                                state.navigate(r.path.clone());
                                state.path == r.path
                            } else {
                                places_sidebar::locate(&mut state, &r.path)
                            };
                            eprintln!(
                                "FILES_TOOLKIT_RECENT_PICK index={i} path={} located={ok}",
                                r.path.display()
                            );
                        }
                        let mut canvas = c.clone();
                        let _ = canvas.take_focus();
                        canvas.redraw();
                    });
                }
                Err(e) => notice(&format!("No se pudieron leer los recientes: {e}")),
            }
        },
    )
}

fn open_recent(
    loader: &fltk_omarchy::task::Latest<Result<Vec<files_core::recent::Recent>, String>>,
) {
    let dir = files_core::apps::data_home();
    loader.request(move |ctx| {
        ctx.check()?;
        let shared = files_core::ofolder::read_recents(
            &files_core::ofolder::Paths::from_env().recents,
            files_core::recent::MAX_ITEMS,
        )
        .into_iter()
        .filter(|r| {
            std::fs::metadata(&r.path)
                .is_ok_and(|m| if r.folder { m.is_dir() } else { m.is_file() })
        })
        .collect();
        let result = files_core::recent::read(&dir)
            .map(|list| ofolder_ui::merge_recents(list, shared))
            .map_err(|e| e.to_string());
        ctx.check()?;
        Ok(result)
    });
}

pub fn run() -> bool {
    run_since(Instant::now())
}
pub fn run_since(start: Instant) -> bool {
    let mut path = None;
    let mut size = (1100, 700);
    let mut scale = None;
    let mut auto_exit = None;
    let mut list = false;
    let mut spatial = false;
    let mut smoke = false;
    let mut scene = String::new();
    let mut args = std::env::args_os().skip(1);
    let mut end_options = false;
    let mut no_session = false;
    let mut file_manager_service = false;
    let mut external_method = None;
    let mut external_operands = Vec::new();
    while let Some(a) = args.next() {
        if end_options {
            let operand = PathBuf::from(a);
            external_operands.push(operand.clone());
            path = Some(operand);
            continue;
        }
        match a.to_str() {
            Some("--") => end_options = true,
            Some("--size") => {
                if let Some(s) = args.next().and_then(|s| s.into_string().ok())
                    && let Some((w, h)) = s.split_once('x')
                {
                    size = (w.parse().unwrap_or(1100), h.parse().unwrap_or(700));
                }
            }
            Some("--scale") => {
                scale = args
                    .next()
                    .and_then(|s| s.into_string().ok())
                    .and_then(|s| s.parse::<f32>().ok())
            }
            Some("--exit-after") => {
                auto_exit = args
                    .next()
                    .and_then(|s| s.into_string().ok())
                    .and_then(|s| s.parse::<u64>().ok())
            }
            Some("--list") => list = true,
            Some("--spatial" | "--columns") => spatial = true,
            Some("--ui-smoke") => smoke = true,
            Some("--ui-scene") => {
                scene = args
                    .next()
                    .and_then(|s| s.into_string().ok())
                    .unwrap_or_default()
            }
            Some("--no-session") => no_session = true,
            Some("--file-manager-service") => file_manager_service = true,
            Some(mode @ ("--select" | "--show-folders" | "--properties")) => {
                if external_method.is_some() {
                    eprintln!("Usa solo uno de --select, --show-folders o --properties");
                    return false;
                }
                external_method = Some(match mode {
                    "--select" => files_core::file_manager::Method::ShowItems,
                    "--properties" => files_core::file_manager::Method::ShowItemProperties,
                    _ => files_core::file_manager::Method::ShowFolders,
                });
            }
            Some("--version") => {
                println!("files {}", env!("CARGO_PKG_VERSION"));
                return true;
            }
            Some("--help") => {
                println!(
                    "files [carpeta] [--select rutas... | --show-folders rutas... | --properties rutas...] [--file-manager-service] [--size 1100x700] [--scale 1] [--list|--columns] [--exit-after segundos] [--no-session] [--ui-smoke]"
                );
                return true;
            }
            _ if !a.as_encoded_bytes().starts_with(b"-") => {
                let operand = PathBuf::from(a);
                external_operands.push(operand.clone());
                path = Some(operand);
            }
            _ => {
                eprintln!("Opción desconocida: {a:?}");
                return false;
            }
        }
    }
    if file_manager_service && (external_method.is_some() || !external_operands.is_empty()) {
        eprintln!(
            "El servicio FileManager1 no admite una ubicación inicial; usa --select o --show-folders por separado"
        );
        return false;
    }
    if external_method.is_none()
        && (external_operands.len() > 1
            || external_operands
                .first()
                .and_then(|p| p.to_str())
                .is_some_and(|s| {
                    s.get(..7)
                        .is_some_and(|p| p.eq_ignore_ascii_case("file://"))
                }))
    {
        external_method = Some(files_core::file_manager::Method::ShowFolders);
    }
    let external_request = if let Some(method) = external_method {
        let cwd = match std::env::current_dir() {
            Ok(path) => path,
            Err(error) => {
                eprintln!("{error}");
                return false;
            }
        };
        match file_manager_ui::cli_request(method, &external_operands, &cwd) {
            Ok(request) => Some(request),
            Err(error) => {
                eprintln!("{error}");
                return false;
            }
        }
    } else {
        None
    };
    let mut waiting_for_external = file_manager_service || external_request.is_some();
    if waiting_for_external {
        path = None;
        no_session = true;
    }
    let explicit_path = path.is_some() || waiting_for_external;
    let path = path.unwrap_or_else(files_core::theme::home);
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    if let Err(error) = files_core::navigation_history::validate(&path) {
        eprintln!("{error}");
        return false;
    }
    let capture_path = if smoke {
        std::env::var_os("FILES_TEST_CAPTURE").map(PathBuf::from)
    } else {
        None
    };
    let trace = std::env::var_os("FILES_TRACE").is_some();
    let metrics_readiness = std::env::var_os("FILES_METRICS_READY").is_some();
    let mut readiness_emitted = None;
    let _application = app::App::default();
    fltk_omarchy::focus::install();
    if smoke {
        fltk_omarchy::testkit::arm();
    }
    if trace {
        eprintln!("FILES_INIT_US={}", start.elapsed().as_micros());
    }
    if let Some(s) = scale {
        app::set_screen_scale(0, s);
    }

    if trace {
        eprintln!("FILES_PRE_MODEL_US={}", start.elapsed().as_micros());
    }
    let font_start = Instant::now();
    let initial_toolkit = fltk_omarchy::theme::Theme::try_load().unwrap_or_else(|error| {
        eprintln!("FILES_THEME_TOOLKIT_REJECTED initial=true error={error}");
        fltk_omarchy::theme::Theme::from_tables(
            "Fallback",
            &fltk_omarchy::toml::parse(fltk_omarchy::theme::FALLBACK_COLORS),
            &Default::default(),
            0,
            5,
        )
    });
    fltk_omarchy::theme::set(initial_toolkit);
    let model = Rc::new(RefCell::new(Model::new(path)));
    fltk_omarchy::theme::set_corners(omarchy_theme::corners(model.borrow().corners));
    fonts::configure();
    let theme_override = std::env::var_os("FILES_THEME").is_some();
    if !theme_override {
        omarchy_theme::apply(&mut model.borrow_mut());
    }
    model.borrow_mut().start_devices();
    model.borrow_mut().start_network();
    model.borrow_mut().start_ofolder();
    // Scripted media checks were written for a paused, muted first frame.
    // They keep it unless they ask for the product behaviour (autoplay and,
    // with no saved preference, sound on).
    if smoke {
        let mut m = model.borrow_mut();
        if std::env::var_os("FILES_TEST_VIDEO_AUTOPLAY").as_deref()
            != Some(std::ffi::OsStr::new("1"))
        {
            m.preview.set_autoplay(false);
            if !m.preview_muted_configured {
                m.preview.set_muted(true);
            }
        }
    }
    // Scripted checks compare pixels, so they paint opaque whatever the
    // desktop's Window Opacity is, unless they ask with OMARCHY_FLTK_GLASS.
    let glass_pinned = smoke && std::env::var_os("OMARCHY_FLTK_GLASS").is_none();
    // `smoke` is cleared once the smoke actions ran; the exit report is not.
    let glass_report = smoke || trace;
    if glass_pinned {
        fltk_omarchy::glass::set(fltk_omarchy::glass::Setting {
            mode: fltk_omarchy::glass::Mode::Opaque,
            opacity: 1.0,
            overridden: true,
        });
    }
    let toolkit_changed = Rc::new(Cell::new(false));
    let change_flag = toolkit_changed.clone();
    fltk_omarchy::theme::on_change(move |_| change_flag.set(true));
    if trace {
        eprintln!("FILES_FONT_US={}", font_start.elapsed().as_micros());
    }
    let session_disabled =
        no_session || (smoke && std::env::var_os("FILES_TEST_SESSION").is_none());
    if session_disabled {
        model.borrow_mut().session.mode = session_state::Mode::Disabled;
    } else if !explicit_path {
        model.borrow_mut().begin_session(true, false);
    }
    if trace {
        eprintln!("FILES_MODEL_US={}", start.elapsed().as_micros());
    }
    if list {
        model.borrow_mut().view = View::List;
    }
    if spatial || scene.starts_with("spatial") {
        model.borrow_mut().action(Action::View(View::Columns));
    }
    let mut win = Window::default()
        .with_size(size.0, size.1)
        .with_label("files");
    win.set_xclass("files");
    app_identity::set_icon(&mut win);
    win.set_label(&app_identity::title(&model.borrow()));
    win.set_border(true);
    if smoke
        && std::env::var_os("FILES_TEST_FIXED_SIZE").as_deref() == Some(std::ffi::OsStr::new("1"))
    {
        win.size_range(size.0, size.1, size.0, size.1);
    } else {
        win.size_range(700, 440, 0, 0);
    }
    let isolate_fixture_input = smoke
        && std::env::var_os("FILES_TEST_ISOLATE_INPUT").as_deref()
            == Some(std::ffi::OsStr::new("1"));
    let fixture_dispatch = Rc::new(Cell::new(false));
    let mut canvas = Frame::new(0, 0, size.0, size.1, "");
    canvas.set_frame(FrameType::NoBox);
    canvas.set_label("");
    // These invisible widgets carry the geometry of the painted toolbar
    // buttons; PopupMenu uses them as anchors, not as additional hit targets.
    let mut settings_anchor = Frame::new(0, 0, 1, 1, "");
    let mut actions_anchor = Frame::new(0, 0, 1, 1, "");
    settings_anchor.hide();
    actions_anchor.hide();
    // Places rail: the toolkit Sidebar over the canvas strip (y 90 to the
    // bottom of the browser). It is drawn after the canvas on every paint.
    let recent = recent_loader(&win, &model, &canvas);
    let rail_selected = fltk_omarchy::State::new(usize::MAX);
    let rail_state = Rc::new(RefCell::new(places_sidebar::Rail::default()));
    let mut sidebar = {
        let run = |m: &Rc<RefCell<Model>>, canvas: &Frame, action: Option<Action>| {
            if let Some(action) = action
                && let Ok(mut state) = m.try_borrow_mut()
            {
                state.action(action);
            }
            let mut canvas = canvas.clone();
            let _ = canvas.take_focus();
            canvas.redraw();
        };
        let (m1, r1, c1) = (model.clone(), rail_state.clone(), canvas.clone());
        let recent = recent.clone();
        let (m2, r2, c2) = (model.clone(), rail_state.clone(), canvas.clone());
        fltk_omarchy::widgets::Sidebar::new(Vec::new(), &rail_selected)
            .widths(places_sidebar::RAIL, places_sidebar::WIDE)
            .on_activate(move |i| {
                if r1.borrow().kinds.get(i) == Some(&places_sidebar::Kind::Recent) {
                    open_recent(&recent);
                    return;
                }
                let action = r1.borrow().actions.get(i).cloned().flatten();
                run(&m1, &c1, action);
            })
            .on_secondary(move |i| {
                let action = r2.borrow().secondary.get(i).cloned().flatten();
                run(&m2, &c2, action);
            })
            // Drops reach Files through its own native transfer (copy review);
            // the FLTK DnD path of the toolkit stays out of it.
            .accept_drop(|_| false)
    };
    // Right click, Menu or Shift+F10 on a row: its menu (open, new tab,
    // remove bookmark, mount / unmount / eject, trash).
    {
        let menu_tree = sidebar.tree();
        let (m3, r3, c3) = (model.clone(), rail_state.clone(), canvas.clone());
        let recent = recent.clone();
        sidebar = sidebar.on_context(move |i, x, y| {
            let entries = match m3.try_borrow() {
                Ok(state) => places_sidebar::menu(&state, &r3.borrow(), i),
                Err(_) => return,
            };
            if entries.is_empty() {
                return;
            }
            let (items, picks): (Vec<_>, Vec<_>) = entries.into_iter().unzip();
            let (m4, c4) = (m3.clone(), c3.clone());
            let recent = recent.clone();
            eprintln!("FILES_TOOLKIT_SIDEBAR_MENU row={i} entries={}", picks.len());
            menu_tree.open_menu(x, y, items, move |k| {
                if let Some(pick) = picks.get(k).cloned() {
                    eprintln!("FILES_TOOLKIT_SIDEBAR_PICK row={i} entry={k} pick={pick:?}");
                    if matches!(pick, places_sidebar::Pick::OpenRecent) {
                        open_recent(&recent);
                        return;
                    }
                    if let Ok(mut state) = m4.try_borrow_mut() {
                        places_sidebar::apply(&mut state, pick);
                    }
                }
                let mut canvas = c4.clone();
                let _ = canvas.take_focus();
                canvas.redraw();
            });
        });
    }
    // Rows of places, devices and bookmarks reorder by dragging within their
    // section. With reordering on, the toolkit activates a row on a release
    // that did not drag, so a drag never navigates.
    {
        let (m5, r5, c5) = (model.clone(), rail_state.clone(), canvas.clone());
        let _ = sidebar
            .tree()
            .reorder(fltk_omarchy::navtree::Reorder::Siblings)
            .accept_move(|key, parent, _| {
                places_sidebar::ORDERED_SECTIONS.contains(&parent) && !key.starts_with("ofav:")
            })
            .on_move(move |key, parent, index| {
                let pick = match m5.try_borrow() {
                    Ok(state) => places_sidebar::drag_pick(&state, &r5.borrow(), key, parent, index),
                    Err(_) => return,
                };
                eprintln!("FILES_TOOLKIT_SIDEBAR_MOVE key={key} parent={parent} index={index} pick={pick:?}");
                if let Some(pick) = pick
                    && let Ok(mut state) = m5.try_borrow_mut()
                {
                    places_sidebar::apply(&mut state, pick);
                }
                c5.clone().redraw();
            });
    }
    sidebar.resize(0, 90, places_sidebar::RAIL, (size.1 - 122).max(0));
    // The rail joins the Tab order only on an explicit keyboard request (Tab
    // in a single pane, Ctrl+Alt+S, palette); a pointer press never keeps
    // keyboard focus there.
    sidebar.clear_visible_focus();
    let fixture_sidebar = sidebar.clone();
    let fixture_rail = rail_state.clone();
    let tab_strip = tab_strip::Strip::new(size.0, isolate_fixture_input, fixture_dispatch.clone());
    let mut operation_notices = operation_notices::Manager::default();
    let mut mutation_sheets = mutation_sheets::Manager::default();
    let close_requested = Rc::new(Cell::new(false));
    let mut window_presentation = window_presentation::Manager::new(close_requested.clone());
    let notice_input = operation_notices.inbox();
    model.borrow_mut().toolkit_feedback = true;
    win.end();
    win.resizable(&canvas);
    if trace {
        eprintln!("FILES_WINDOW_US={}", start.elapsed().as_micros());
    }
    let paint_revision = Rc::new(Cell::new(0u64));
    let presentation_key_events = Rc::new(Cell::new(0u64));
    let observe_presentation = smoke && std::env::var_os("FILES_TEST_TOOLKIT_POPUP").is_some();
    let drawn_revision = paint_revision.clone();
    let painted_size = Rc::new(Cell::new((0i32, 0i32)));
    let drawn_size = painted_size.clone();
    let m = model.clone();
    let mut painted = false;
    let scale_window = win.clone();
    let rail_window = win.clone();
    let mut rail_sidebar = sidebar.clone();
    let mut rail_motion = fltk_omarchy::anim::Anim::<f64>::new();
    let rail_selected = rail_selected.clone();
    let fixture_rail_state = rail_state.clone();
    let rail_state = rail_state.clone();
    let mut painted_tabs = tab_strip.clone();
    let tabs_window = win.clone();
    canvas.draw(move |f| {
        if !painted && trace {
            eprintln!("FILES_PAINT_START_US={}", start.elapsed().as_micros());
        }
        let mut p = painter::FltkPainter;
        // Includes the Wayland output scale (2 on a Retina/HiDPI output).
        painter::set_device_scale(fltk_omarchy::native::pixels_per_unit(&scale_window));
        let mut m = m.borrow_mut();
        if f.damage_type() == Damage::User1 {
            view::status(&mut p, &mut m, f.w(), f.h());
        } else {
            crate::wheel_scroll::advance(&mut m, f);
            crate::trash_browser::advance(&mut m, f);
            let covered = places_sidebar::covered(&m);
            if let Some(pin) = m.rail_request.take() {
                rail_sidebar.set_pinned(pin);
            }
            m.sidebar_widget = !covered;
            if !covered {
                m.rail = rail_sidebar.is_expanded();
            }
            let target = if m.rail {
                places_sidebar::WIDE
            } else {
                places_sidebar::RAIL
            };
            // A free canvas has no Flex geometry to animate. Share the toolkit
            // clock/timer and use this one sample for the widget and all content.
            let width = if covered {
                rail_motion.jump(target as f64)
            } else {
                rail_motion.get(
                    target as f64,
                    fltk_omarchy::anim::ms::LAYOUT,
                    fltk_omarchy::anim::Ease::OutCubic,
                    f,
                )
            }
            .round() as i32;
            m.rail_width = Some(width.clamp(places_sidebar::RAIL, places_sidebar::WIDE));
            painted_tabs.sync(&mut m, f.w());
            let previous_scroll = m.scroll;
            view::paint(&mut p, &mut m, f.w(), f.h());
            m.toolbar.schedule(f);
            m.clamp_scroll(f.h() - 134);
            if previous_scroll != m.scroll {
                view::paint(&mut p, &mut m, f.w(), f.h());
            }
            if covered {
                m.rail_drops.clear();
                if rail_sidebar.visible() {
                    rail_sidebar.hide();
                }
            } else {
                let rail = places_sidebar::rail(&m, m.rail_pinned, &m.rail_order);
                // A row moved by dragging that did not end up saved (or any
                // other drift) is put back once no bookmark request runs.
                let drifted = !m.bookmarks.busy() && {
                    let mut shown = Vec::new();
                    rail_sidebar
                        .tree()
                        .model()
                        .for_each(|_, it| shown.push(it.key.clone()));
                    shown
                        .iter()
                        .map(String::as_str)
                        .ne(rail.items.iter().map(|i| i.key.as_str()))
                };
                if drifted || rail.items != rail_state.borrow().items {
                    rail_sidebar.set_items(rail.items.clone());
                }
                rail_selected.set_if_changed(rail.active);
                *rail_state.borrow_mut() = rail;
                let strip = (crate::operation_bar::browser_height(&m, f.h()) - 122).max(0);
                // Repair FLTK's child scaling with the same animated sample
                // used by the browser, rather than forcing an endpoint.
                let w = m.rail_width();
                if (
                    rail_sidebar.x(),
                    rail_sidebar.y(),
                    rail_sidebar.w(),
                    rail_sidebar.h(),
                ) != (0, 90, w, strip)
                {
                    rail_sidebar.resize(0, 90, w, strip);
                }
                if !rail_sidebar.visible() {
                    rail_sidebar.show();
                }
                // Plain FLTK redraws damaged siblings independently: the
                // canvas just painted the strip, so the rail goes on top now.
                rail_window.draw_child(&mut *rail_sidebar);
                let drops: Vec<_> = (0..rail_state.borrow().items.len())
                    .filter_map(|i| {
                        let dir = rail_state.borrow().drop_dir(i)?.to_path_buf();
                        let (x, y, w, h) = rail_sidebar.item_rect(i)?;
                        Some((painter::Rect::new(x, y, w, h), dir))
                    })
                    .collect();
                m.rail_drops = drops;
                // The drop highlight of a rail row goes over the widget.
                if m.native_transfer
                    .hover()
                    .is_some_and(|t| matches!(t.scope, native_drop::Scope::Place))
                {
                    native_drop::paint(&mut p, &m);
                }
            }
            painted_tabs.paint(&mut m, &tabs_window);
            toolbar::paint_tooltip(&mut p, &mut m, f.w());
            drawn_size.set((f.w(), f.h()));
            drawn_revision.set(drawn_revision.get().wrapping_add(1));
            motion::schedule(&m, f);
        }
        if !painted {
            eprintln!("FILES_FIRST_PAINT_US={}", start.elapsed().as_micros());
            painted = true;
        }
    });
    // Toolkit fixture: an invisible widget over a toolbar control, so the
    // toolkit tooltip can be shown without OS pointer input.
    let mut tooltip_probe: Option<Frame> = None;
    let fixture_dispatch_event = fixture_dispatch.clone();
    let m = model.clone();
    let mut gesture_window = win.clone();
    let mut grabbing = None;
    let mut deferred_click = None;
    let event_tabs = tab_strip.clone();
    let event_notices = notice_input.clone();
    // Keyboard focus in the places rail: the toolkit Sidebar takes it only on
    // request (Tab, Ctrl+Alt+S, palette) and gives it back to the list.
    let rail_cursor: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let event_sidebar = sidebar.clone();
    let event_rail_cursor = rail_cursor.clone();
    canvas.handle(move |f, e| {
        if e == Event::Focus {
            // Back in the list: the rail leaves the Tab order again so a
            // pointer press on it never keeps keyboard focus there.
            event_sidebar.clone().clear_visible_focus();
            event_rail_cursor.set(None);
        }
        // A native modal owns text and navigation. In particular, a late
        // canvas paste must not bypass the sheet's stamped admission queue.
        if m.try_borrow().is_ok_and(|m| m.toolkit_mutation_sheet_open)
            && matches!(
                e,
                Event::Push
                    | Event::Drag
                    | Event::Released
                    | Event::KeyDown
                    | Event::KeyUp
                    | Event::Shortcut
                    | Event::Paste
                    | Event::MouseWheel
            )
        {
            if e == Event::Paste
                && let Ok(mut state) = m.try_borrow_mut()
            {
                // Drain an old asynchronous FL_PASTE receipt without
                // inserting it into the native editor or a later form.
                state.text_paste.invalidate();
                state.text_paste.receive(&None);
            }
            return true;
        }
        let release_click = if e == Event::Released {
            deferred_click.take()
        } else {
            None
        };
        if matches!(e, Event::Push | Event::KeyDown | Event::Shortcut) {
            deferred_click = None;
        }
        if e == Event::Released {
            native_transfer::release_before_borrow(&m);
        }
        if matches!(
            e,
            Event::DndEnter | Event::DndDrag | Event::DndRelease | Event::DndLeave
        ) {
            return false;
        }
        // Explicit fixture-only isolation: handlers below are still called by
        // the scene, but ambient desktop interaction cannot navigate its data.
        if isolate_fixture_input
            && !fixture_dispatch_event.get()
            && matches!(
                e,
                Event::Push
                    | Event::Drag
                    | Event::Released
                    | Event::KeyDown
                    | Event::KeyUp
                    | Event::Shortcut
                    | Event::MouseWheel
                    | Event::Move
                    | Event::Enter
                    | Event::Leave
            )
        {
            return true;
        }

        if matches!(e, Event::KeyDown | Event::Shortcut)
            && app::event_key() == Key::from_char('n')
            && app::event_state().contains(Shortcut::Ctrl | Shortcut::Alt)
        {
            let allowed = m
                .try_borrow()
                .is_ok_and(|m| operation_notices::context(&m).visible);
            if allowed {
                event_notices.focus();
            }
            return true;
        }
        if matches!(e, Event::KeyDown | Event::Shortcut)
            && app::event_key() == Key::from_char('t')
            && app::event_state().contains(Shortcut::Ctrl | Shortcut::Alt)
        {
            let key = m
                .try_borrow_mut()
                .ok()
                .and_then(|mut m| tab_strip::begin_intent(&mut m).then_some(m.pane_owner()));
            if let Some(key) = key {
                event_tabs.widget.focus_key(&key);
            }
            return true;
        }

        let _native_event = native_transfer::EventGuard::begin(&m, &gesture_window, e);
        if (e == Event::KeyDown || e == Event::Shortcut)
            && app::event_state().intersects(Shortcut::Ctrl | Shortcut::Meta)
            && app::event_key() == Key::from_char('v')
        {
            if app::event_state().intersects(Shortcut::Shift) && m.borrow().text_context().is_none()
            {
                m.borrow_mut().action(Action::NativePaste);
            } else {
                crate::clipboard_ui::request_paste(&m, f);
            }
            return true;
        }
        let Ok(mut state) = m.try_borrow_mut() else {
            return false;
        };
        if e == Event::Paste {
            state.receive_text_paste(&app::event_text());
            f.redraw();
            return true;
        }
        if e == Event::Push && app::event_button() == 3 {
            if tab_context::pointer(&mut state, app::event_x(), app::event_y()) {
                f.redraw();
                return true;
            }
            if state.context_menu.is_some() {
                crate::context_menu::dismiss(&mut state);
            } else {
                crate::context_menu::open_at(
                    &mut state,
                    app::event_x(),
                    app::event_y(),
                    f.w(),
                    f.h(),
                );
            }
            f.redraw();
            return true;
        }
        if matches!(
            e,
            Event::Push
                | Event::KeyDown
                | Event::MouseWheel
                | Event::Hide
                | Event::Deactivate
                | Event::Unfocus
        ) && state.toolbar.suppress()
        {
            f.redraw();
        }
        let zone = window_chrome::at(&state, f.w(), f.h(), app::event_x(), app::event_y());
        if matches!(
            e,
            Event::Move | Event::Enter | Event::Leave | Event::Released
        ) {
            if e == Event::Released || e == Event::Leave {
                grabbing = None;
            }
            state.pointer = (e != Event::Leave).then(|| (app::event_x(), app::event_y()));
            window_chrome::cursor(
                &mut gesture_window,
                if e == Event::Leave { None } else { zone },
                false,
            );
            let control_name = state
                .hits
                .iter()
                .rev()
                .find(|hit| {
                    hit.generation == state.generation
                        && hit.rect.contains(app::event_x(), app::event_y())
                })
                .and_then(|hit| {
                    crate::copy_review::reason(&state, &hit.action)
                        .or_else(|| crate::map_actions::reason(&state, &hit.action))
                        .or_else(|| hit.action.accessible_name())
                });
            let tip = if view::tip_rect(f.w(), f.h()).contains(app::event_x(), app::event_y())
                && view::tips_available(&state, f.w())
            {
                state.status_tips.text()
            } else {
                control_name.unwrap_or_else(|| zone.map_or("", |z| z.name()))
            };
            f.set_tooltip(if app::event_y() < 58 { "" } else { tip });
        }
        if e == Event::Push
            && app::event_button() == 1
            && let Some(zone) = zone
        {
            if window_chrome::begin(&mut gesture_window, zone) {
                grabbing = Some(zone);
                window_chrome::cursor(&mut gesture_window, Some(zone), true);
            } else {
                state.error = "Movimiento nativo no disponible en este evento/plataforma".into();
                f.redraw();
            }
            return true;
        }
        if e == Event::Drag && grabbing.is_some() {
            return true;
        }
        if e == Event::Drag && state.outgoing_drag_motion(app::event_x(), app::event_y()) {
            deferred_click = None;
            f.redraw();
            return true;
        }
        let plain_press = e == Event::Push
            && app::event_button() == 1
            && !app::event_clicks()
            && !app::event_state().intersects(Shortcut::Ctrl | Shortcut::Meta | Shortcut::Shift);
        if plain_press
            && state.selection.len() > 1
            && let Some(i) = state.drag_selected_hit(app::event_x(), app::event_y())
            && state.arm_outgoing_drag(app::event_x(), app::event_y())
        {
            deferred_click = Some((state.pane_owner(), state.generation, i));
            state.session.restore_allowed = false;
            f.redraw();
            return true;
        }
        if e == Event::Released
            && let Some((owner, generation, index)) = release_click
            && state.pane_owner() == owner
            && state.generation == generation
        {
            state.select_entry(index, false, false);
        }
        let handled = event(&mut state, e, f.w(), f.h());
        if plain_press {
            state.arm_outgoing_drag(app::event_x(), app::event_y());
        }
        let focus = state.focus_request.take();
        let tabs_key = (focus == Some(key_commands::FocusTarget::Tabs)
            && tab_strip::begin_intent(&mut state))
        .then(|| state.pane_owner());
        let rail_open = state.sidebar_widget && !places_sidebar::covered(&state);
        drop(state);
        match focus {
            Some(key_commands::FocusTarget::Tabs) if tabs_key.is_some() => {
                event_tabs.widget.focus_key(&tabs_key.expect("tab key"));
            }
            Some(key_commands::FocusTarget::Tabs | key_commands::FocusTarget::Rail)
                if rail_open =>
            {
                let mut rail = event_sidebar.clone();
                rail.set_visible_focus();
                if rail.take_focus().is_ok() {
                    event_rail_cursor.set(Some(rail.cursor()));
                    rail.redraw();
                } else {
                    rail.clear_visible_focus();
                }
            }
            _ => {}
        }
        if handled {
            f.redraw();
        }
        handled
    });
    let native_window_model = model.clone();
    let (aux_sidebar, aux_rail, mut aux_canvas) =
        (sidebar.clone(), fixture_rail.clone(), canvas.clone());
    let auxiliary_fixture = fixture_dispatch.clone();
    let auxiliary_tabs = tab_strip.clone();
    let auxiliary_notices = notice_input.clone();
    let auxiliary_rail_cursor = rail_cursor.clone();
    let mut auxiliary_pressed = false;
    // FLTK groups otherwise dispatch children before our handler. Intercept
    // auxiliary buttons before the canvas or Sidebar treats them as primary.
    win.super_handle_first(false);
    let observed_presentation_keys = presentation_key_events.clone();
    win.handle(move |window, event| {
        if observe_presentation && event == Event::KeyDown {
            observed_presentation_keys.set(observed_presentation_keys.get().wrapping_add(1));
        }
        native_transfer::window_event(&native_window_model, window, event);
        if native_window_model
            .try_borrow()
            .is_ok_and(|m| m.toolkit_mutation_sheet_open)
        {
            return false;
        }
        if matches!(event, Event::KeyDown | Event::Shortcut) {
            let mods = app::event_state();
            if app::event_key() == Key::from_char('n')
                && mods.contains(Shortcut::Ctrl | Shortcut::Alt)
                && (!isolate_fixture_input || auxiliary_fixture.get())
            {
                let allowed = native_window_model
                    .try_borrow()
                    .is_ok_and(|m| operation_notices::context(&m).visible);
                if allowed {
                    auxiliary_notices.focus();
                }
                return true;
            }
            if app::event_key() == Key::F10
                && mods.contains(Shortcut::Ctrl | Shortcut::Shift)
                && (!isolate_fixture_input || auxiliary_fixture.get())
                && let Ok(mut m) = native_window_model.try_borrow_mut()
            {
                auxiliary_tabs.active_context(&mut m);
                aux_canvas.redraw();
                return true;
            }
            if app::event_key() == Key::from_char('t')
                && mods.contains(Shortcut::Ctrl | Shortcut::Alt)
                && (!isolate_fixture_input || auxiliary_fixture.get())
                && let Ok(mut m) = native_window_model.try_borrow_mut()
                && tab_strip::begin_intent(&mut m)
            {
                auxiliary_tabs.widget.focus_key(&m.pane_owner());
                return true;
            }
            if (!isolate_fixture_input || auxiliary_fixture.get())
                && event == Event::KeyDown
                && app::event_key() == Key::Escape
                && app::focus()
                    .is_some_and(|w| w.as_widget_ptr() == auxiliary_tabs.widget.as_widget_ptr())
            {
                // A pending drag gets Escape first. Otherwise return to the
                // browser without clearing its independent selection.
                let mut widget = auxiliary_tabs.widget.clone();
                if !widget.handle_event(Event::KeyDown) {
                    let _ = aux_canvas.take_focus();
                }
                return true;
            }
        }
        // Keys the focused places rail leaves unused bubble up here (the
        // toolkit already moved its cursor, activated rows, opened the row
        // menu or ran a row's trailing action).
        if matches!(event, Event::KeyDown | Event::KeyUp | Event::Shortcut)
            && (!isolate_fixture_input || auxiliary_fixture.get())
            && app::focus().is_some_and(|w| w.as_widget_ptr() == aux_sidebar.as_widget_ptr())
        {
            let cursor = aux_sidebar.cursor();
            let to_list = |canvas: &mut Frame| {
                let _ = canvas.take_focus();
                canvas.redraw();
            };
            match event {
                Event::KeyDown => {
                    let command = match native_window_model.try_borrow() {
                        Ok(m) => key_commands::rail_key(
                            &m,
                            &aux_rail.borrow(),
                            cursor,
                            app::event_key(),
                            app::event_state(),
                        ),
                        Err(_) => return true,
                    };
                    match command {
                        key_commands::RailKey::ReturnToList => to_list(&mut aux_canvas),
                        key_commands::RailKey::Pick(pick) => {
                            let opens_tab = matches!(pick, places_sidebar::Pick::Run(_));
                            if let Ok(mut m) = native_window_model.try_borrow_mut() {
                                places_sidebar::apply(&mut m, pick);
                            }
                            if opens_tab {
                                to_list(&mut aux_canvas);
                            }
                            aux_canvas.redraw();
                        }
                        key_commands::RailKey::Background(path) => {
                            if let Ok(mut m) = native_window_model.try_borrow_mut() {
                                m.action(Action::BackgroundNavigate(path));
                            }
                            aux_canvas.redraw();
                        }
                        key_commands::RailKey::Pass => {
                            // The list handles the chord as a shortcut next.
                            to_list(&mut aux_canvas);
                            return false;
                        }
                        key_commands::RailKey::Consume => {}
                    }
                    return true;
                }
                Event::KeyUp => {
                    // The toolkit uses Esc to fold the expanded rail and Right
                    // to enter a section; once released they return to the
                    // list unless Right just moved the cursor or sits on a
                    // section header.
                    let before = auxiliary_rail_cursor.replace(Some(cursor));
                    let header = matches!(
                        aux_rail.borrow().kinds.get(cursor),
                        Some(places_sidebar::Kind::Header)
                    );
                    let key = app::event_key();
                    if key == Key::Escape
                        || (key == Key::Right && before == Some(cursor) && !header)
                    {
                        to_list(&mut aux_canvas);
                    }
                    return true;
                }
                // Arrows the rail did not use are offered as shortcuts; they
                // must not move the file list behind it.
                _ => return true,
            }
        }
        if event == Event::Push {
            auxiliary_pressed = false;
            if app::event_button() != 1
                && auxiliary_notices.contains(app::event_x(), app::event_y())
            {
                auxiliary_pressed = true;
                return true;
            }
            if auxiliary_tabs.contains(app::event_x(), app::event_y())
                && (!isolate_fixture_input || auxiliary_fixture.get())
                && let Ok(mut m) = native_window_model.try_borrow_mut()
            {
                tab_strip::begin_intent(&mut m);
            }
            let button = app::event_button();
            if button != 1 && button != 3 {
                if auxiliary_tabs.contains(app::event_x(), app::event_y()) {
                    if isolate_fixture_input && !auxiliary_fixture.get() {
                        return true;
                    }
                    // The keyed widget owns press/release and cancellation here.
                    return false;
                }
                auxiliary_pressed = true;
                if isolate_fixture_input && !auxiliary_fixture.get() {
                    return true;
                }
                if button == 2
                    && let Ok(mut m) = native_window_model.try_borrow_mut()
                {
                    let (x, y) = (app::event_x(), app::event_y());
                    let target = if aux_sidebar.visible()
                        && x >= aux_sidebar.x()
                        && x < aux_sidebar.x() + aux_sidebar.w()
                        && y >= aux_sidebar.y()
                        && y < aux_sidebar.y() + aux_sidebar.h()
                    {
                        let rail = aux_rail.borrow();
                        (0..rail.items.len()).find_map(|i| {
                            let (rx, ry, rw, rh) = aux_sidebar.item_rect(i)?;
                            if x >= rx && x < rx + rw && y >= ry && y < ry + rh {
                                rail.drop_dir(i).map(|p| Action::Navigate(p.to_path_buf()))
                            } else {
                                None
                            }
                        })
                    } else {
                        tab_gestures::hit(&m, x, y)
                    };
                    tab_gestures::middle(&mut m, target);
                    aux_canvas.redraw();
                }
                // No auxiliary button may trigger a primary/destructive action.
                return true;
            }
        }
        if auxiliary_pressed && matches!(event, Event::Drag | Event::Released) {
            if event == Event::Released {
                auxiliary_pressed = false;
            }
            return true;
        }
        false
    });
    let close = close_requested.clone();
    let close_trace = std::env::var_os("FILES_CLOSE_TRACE").is_some();
    win.set_callback(move |_| {
        if close_trace {
            eprintln!("FILES_CLOSE_CALLBACK source=fltk_callback");
        }
        close.set(true);
    });
    let mut close_marker = String::new();
    let mut watcher = theme_watch::ThemeWatch::new(!theme_override);
    let mut toolkit_menus = toolkit_menu::Manager::default();
    let toolkit_menu_enabled = !smoke || std::env::var_os("FILES_TEST_TOOLKIT_POPUP").is_some();
    let toolkit_fixture = smoke && std::env::var_os("FILES_TEST_TOOLKIT_POPUP").is_some();
    // Fixtures use the same plain-FLTK initialization as the application.
    // Overlays must initialize their own repaint protection.
    let mut spatial_probe_done = false;
    let mut du_probe_done = false;
    let mut rename_preview_emitted = None;
    let mut rename_status_emitted = String::new();
    let mut preview_emitted = String::new();
    let preview_fixture = matches!(
        scene.as_str(),
        "text" | "image" | "markdown" | "html" | "video" | "pdf" | "audio" | "folder" | "archive"
    ) && smoke
        && std::env::var_os("FILES_TEST_PREVIEW").as_deref() == Some(std::ffi::OsStr::new("1"));
    let copy_fixture = scene == "copy"
        && smoke
        && std::env::var_os("FILES_TEST_COPY").as_deref() == Some(std::ffi::OsStr::new("1"));
    let copy_fixture_root = model
        .borrow()
        .path
        .parent()
        .map(std::path::Path::to_path_buf);
    let window_fixture = scene == "window"
        && smoke
        && std::env::var_os("FILES_TEST_WINDOW").as_deref() == Some(std::ffi::OsStr::new("1"));
    let trash_fixture = scene == "trash"
        && smoke
        && std::env::var_os("FILES_TEST_TRASH").as_deref() == Some(std::ffi::OsStr::new("1"));
    let properties_fixture = scene == "properties"
        && smoke
        && std::env::var_os("FILES_TEST_PROPERTIES").as_deref() == Some(std::ffi::OsStr::new("1"));
    let open_fixture = scene == "open"
        && smoke
        && std::env::var_os("FILES_TEST_OPEN").as_deref() == Some(std::ffi::OsStr::new("1"));
    let hidden_fixture = scene == "hidden-context"
        && smoke
        && std::env::var_os("FILES_TEST_HIDDEN_CONTEXT").is_some();
    let session_fixture = hidden_fixture && std::env::var_os("FILES_TEST_SESSION").is_some();
    let space_fixture = hidden_fixture
        || (scene == "space-map" && smoke && std::env::var_os("FILES_TEST_SPACE_MAP").is_some());
    let space_fixture_root = model.borrow().path.clone();
    let mut space_seq = 0u64;
    let mut space_marker = String::new();
    let thumbnail_fixture = hidden_fixture && std::env::var_os("FILES_TEST_THUMBNAILS").is_some();
    let mut thumbnail_marker = String::new();
    let mut space_painted_after = 0;
    let mut open_seq = 0u64;
    let mut open_reported = None;
    let mut open_painted_after = 0;
    let mut properties_seq = 0u64;
    let mut properties_reported = None;
    let mut properties_painted_after = 0;
    let mut trash_seq = 0u64;
    let mut trash_reported = None;
    let mut window_seq = 0u64;
    let mut window_reported = None;
    let mut copy_marker = String::new();
    let mut move_seq = 0u64;
    let mut move_marker = String::new();
    let mutation_fixture = scene == "rename"
        && smoke
        && std::env::var_os("FILES_TEST_MUTATIONS").as_deref() == Some(std::ffi::OsStr::new("1"));
    let mut identity = app_identity::State::default();
    let mut last_metrics = Instant::now() - Duration::from_secs(2);
    let mut file_manager = match file_manager_ui::Controller::new(file_manager_service) {
        Ok(controller) => controller,
        Err(error) => {
            eprintln!("{error}");
            return false;
        }
    };
    if let Some(request) = external_request
        && let Err(error) = file_manager.enqueue_cli(&model.borrow(), request)
    {
        eprintln!("{error}");
        return false;
    }
    let mut file_manager_status = String::new();
    if trace {
        eprintln!("FILES_PRE_SHOW_US={}", start.elapsed().as_micros());
    }
    if !waiting_for_external {
        win.show();
    }
    if smoke || trace {
        let g = fltk_omarchy::glass::setting();
        eprintln!(
            "FILES_GLASS mode={} opacity={:.2} overridden={} wayland={}",
            g.mode.name(),
            g.opacity,
            g.overridden,
            fltk_omarchy::glass::wayland()
        );
    }
    if !isolate_fixture_input && !waiting_for_external {
        model.borrow_mut().native_transfer.enable(&win);
    }
    let mut applied_window_mode = model.borrow().window_mode;
    window_chrome::apply(&mut win, applied_window_mode);
    if trace {
        eprintln!("FILES_SHOW_US={}", start.elapsed().as_micros());
    }
    if !waiting_for_external {
        let _ = canvas.take_focus();
    }
    fltk_omarchy::app::flush();
    let wait_trace = std::env::var_os("FILES_WAIT_TRACE").is_some();
    let mut runtime_failed = false;
    let mut clean_hidden = false;
    let mut pump_revision = 0u64;
    loop {
        if clean_hidden {
            break;
        }
        if win.was_deleted() || canvas.was_deleted() {
            eprintln!(
                "FILES_BACKEND_WIDGET_LOST phase=before_wait; cancelling and draining without widget access"
            );
            runtime_failed = true;
            break;
        }
        if !win.shown() && !waiting_for_external {
            eprintln!("FILES_BACKEND_WIDGET_LOST phase=unexpected_hidden; cancelling and draining");
            runtime_failed = true;
            break;
        }
        if waiting_for_external && !win.shown() {
            std::thread::sleep(Duration::from_millis(5));
        }
        match platform_wait::wait(0.005) {
            Ok(platform_wait::Outcome::Ready { raw, errno }) => {
                if wait_trace && raw >= 2.0 {
                    eprintln!("FILES_WAIT_RAW={raw} errno_snapshot={errno} success=true");
                }
            }
            Ok(platform_wait::Outcome::Interrupted) => {
                if wait_trace {
                    eprintln!("FILES_WAIT_INTERRUPTED EINTR");
                }
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
            Err(error) => {
                eprintln!("Error real del bucle gráfico: {error}");
                runtime_failed = true;
                break;
            }
        }
        if win.was_deleted() || canvas.was_deleted() {
            eprintln!(
                "FILES_BACKEND_WIDGET_LOST phase=after_wait; cancelling and draining without widget access"
            );
            runtime_failed = true;
            break;
        }
        fltk_omarchy::task::drain();
        pump_revision = pump_revision.wrapping_add(1);
        // Observe the native close latch before an external request can map
        // or activate a window. The callback itself never borrows Model.
        let callback_close = close_requested.replace(false);
        let close_dirty = callback_close
            || auto_exit.is_some_and(|s| start.elapsed() >= Duration::from_secs(s))
                && !model.borrow().closing;
        if close_dirty {
            let mut m = model.borrow_mut();
            m.closing = true;
            if close_trace {
                eprintln!("FILES_CLOSE_REQUEST callback={callback_close}");
            }
            auto_exit = None;
            m.mutations.cancel();
        }
        let feedback_dirty = operation_notices.drain(&model);
        let mutation_sheet_dirty = mutation_sheets
            .controller
            .drain(&mut model.borrow_mut())
            .dirty;
        let desired_corners = omarchy_theme::corners(model.borrow().corners);
        if fltk_omarchy::theme::corners() != desired_corners {
            fltk_omarchy::theme::set_corners(desired_corners);
        }
        let changed = toolkit_changed.replace(false);
        if changed {
            // Toolkit refreshes FLTK font slots; restore Files' resolved Nerd faces.
            fonts::reapply();
        }
        let mut m = model.borrow_mut();
        tab_strip.refresh_hits(&mut m);
        if changed && !theme_override {
            omarchy_theme::apply(&mut m);
            fltk_omarchy::theme::with(|theme| {
                let bg = theme.palette.background;
                eprintln!(
                    "FILES_THEME_TOOLKIT background=#{:02x}{:02x}{:02x} corners={} radius={} font_base={} surface=#{:02x}{:02x}{:02x}",
                    bg.r(),
                    bg.g(),
                    bg.b(),
                    theme.corners.as_str(),
                    theme.radius,
                    theme.font.base,
                    m.system_palette.darker_background.0,
                    m.system_palette.darker_background.1,
                    m.system_palette.darker_background.2
                );
            });
        }
        m.set_media_visible(win.shown() && win.visible_r());
        let strip_dirty = tab_strip.drain(&mut m);
        // Apply queued user gestures before validating presentation, while
        // keeping the initial listing/watch refresh after that validation.
        drop(m);
        window_presentation.pump(&model, &mut win, &mut canvas, paint_revision.get());
        if win.was_deleted() || canvas.was_deleted() {
            eprintln!("FILES_BACKEND_WIDGET_LOST phase=presentation; cancelling and draining");
            runtime_failed = true;
            break;
        }
        let mut m = model.borrow_mut();
        let mut dirty = m.pump()
            || changed
            || strip_dirty
            || feedback_dirty
            || close_dirty
            || mutation_sheet_dirty;
        {
            let Model {
                operation_feedback,
                operation_message,
                ..
            } = &mut *m;
            dirty |= operation_feedback.observe_info(operation_message.as_deref());
        }
        let external = file_manager.pump(&mut m);
        dirty |= external.dirty || file_manager_ui::Controller::finish_reveal(&mut m);
        if let Some(error) = &external.fatal {
            eprintln!("Mostrar en carpeta: {error}");
            runtime_failed = true;
            m.closing = true;
        }
        if file_manager_service {
            let status = file_manager.status();
            if status != file_manager_status {
                eprintln!("FILES_FILE_MANAGER_STATUS {status}");
                file_manager_status = status;
            }
        }
        if m.tab_feedback
            .is_some_and(|at| at.elapsed() >= Duration::from_secs(3))
        {
            m.tab_feedback = None;
            dirty = true;
        }
        if m.window_mode != applied_window_mode {
            applied_window_mode = m.window_mode;
            window_chrome::apply(&mut win, applied_window_mode);
            dirty = true;
        }
        if metrics_readiness && readiness_emitted != Some((m.generation, m.view)) && m.scene_ready()
        {
            let view = match m.view {
                View::Groups => "groups",
                View::List => "list",
                View::Grid => "grid",
                View::Spatial => "spatial",
                View::Columns => "columns",
                View::SpaceMap => "space_map",
            };
            eprintln!(
                "FILES_SCENE_READY view={view} loaded=true entries={} stats_ready={} layout_ready={} generation={}",
                m.entries.len(),
                m.entries
                    .iter()
                    .all(|e| e.stat_attempted && e.metadata.is_some()),
                m.layout_ready,
                m.generation
            );
            readiness_emitted = Some((m.generation, m.view));
        }
        if m.closing
            && let Some(job) = &m.du_job
        {
            job.cancel();
        }
        if m.closing {
            m.pump_media_source();
            if toolkit_menu_enabled {
                toolkit_menus.close();
                toolkit_menus.sync(&mut m, &canvas, &mut settings_anchor, &mut actions_anchor);
            }
        }
        let ready_to_close = m.closing && m.close_ready();
        if m.closing && close_trace {
            let marker = format!(
                "FILES_CLOSE_WAIT saves_ready={ready_to_close} mutations_drained={} copy_closed={} preview_drained={} du_drained={} space_drained={} notice={}",
                m.mutations_drained(),
                m.copy.is_closed(),
                m.preview.is_drained(),
                m.du_job.is_none(),
                m.space_map.is_drained(),
                m.notice_open
            );
            if marker != close_marker {
                eprintln!("{marker}");
                close_marker = marker;
            }
        }
        if m.closing
            && ready_to_close
            && m.du_job.is_none()
            && m.search.is_drained()
            && m.mutations_drained()
            && !m.notice_open
            && file_manager.drained()
        {
            if thumbnail_fixture {
                eprintln!(
                    "FILES_THUMBNAILS_CLOSE allowed={} readonly_pending={} busy={}",
                    m.thumbnails.close_allowed(),
                    m.thumbnails.readonly_pending(),
                    !m.thumbnails.drained()
                );
            }
            if preview_fixture {
                eprintln!(
                    "FILES_MEDIA_PROBE_STOPPED={} scope=readonly-worker-not-required-for-helper-drain",
                    m.media_probe_stopped()
                );
                eprintln!(
                    "FILES_THUMBNAILS_CLOSE allowed={} readonly_pending={}",
                    m.thumbnails.close_allowed(),
                    m.thumbnails.readonly_pending()
                );
                if m.preview.is_drained() {
                    eprintln!("FILES_PREVIEW_DRAINED");
                } else {
                    eprintln!(
                        "FILES_PREVIEW_EXIT_ALLOWED drained=false readonly_pending={} no_future_helper=true",
                        m.preview.readonly_pending()
                    );
                }
            }
            if copy_fixture {
                eprintln!("FILES_COPY_DRAINED");
            }
            if window_fixture {
                eprintln!("FILES_WINDOW_DRAINED");
            }
            if trash_fixture {
                eprintln!("FILES_TRASH_DRAINED");
            }
            if mutation_fixture {
                eprintln!("FILES_MUTATIONS_DRAINED");
            }
            if properties_fixture {
                eprintln!("FILES_PROPERTIES_DRAINED");
            }
            if space_fixture {
                eprintln!("FILES_SPACE_DRAINED");
            }
            if close_trace {
                eprintln!("FILES_CLOSE_DRAINED");
            }
            clean_hidden = true;
            drop(m);
            operation_notices.close();
            mutation_sheets.close();
            window_presentation.close();
            win.hide();
            continue;
        }
        if watcher.changed() {
            if !glass_pinned && fltk_omarchy::glass::reload() {
                let g = fltk_omarchy::glass::setting();
                eprintln!(
                    "FILES_GLASS_CHANGED mode={} opacity={:.2} active={}",
                    g.mode.name(),
                    g.opacity,
                    fltk_omarchy::glass::active()
                );
                dirty = true;
            }
            if theme_override {
                let valid = m.reload_theme();
                dirty = true;
                let c = m.system_palette.background;
                eprintln!(
                    "FILES_THEME_{} background=#{:02x}{:02x}{:02x}",
                    if valid { "RELOADED" } else { "REJECTED" },
                    c.0,
                    c.1,
                    c.2
                );
            } else {
                fltk_omarchy::theme::reload();
                if let Some(error) = fltk_omarchy::theme::last_error() {
                    eprintln!("FILES_THEME_TOOLKIT_REJECTED initial=false error={error}");
                }
            }
        }
        let tooltip_active =
            win.shown() && win.visible() && !window_chrome::blocked(&m) && !m.editing;
        dirty |= m.toolbar.tick(Instant::now(), tooltip_active);
        identity.sync(&m, &mut win);
        if last_metrics.elapsed() >= Duration::from_secs(2) {
            m.pss = files_core::system::pss_kib();
            last_metrics = Instant::now();
            canvas.set_damage_area(Damage::User1, 0, canvas.h() - 32, canvas.w(), 32);
        }
        let tip_active =
            view::tips_available(&m, canvas.w()) && window_chrome::animation_visible(&win);
        if m.status_tips.tick(Instant::now(), tip_active) && !dirty {
            canvas.set_damage_area(Damage::User1, 0, canvas.h() - 32, canvas.w(), 32);
        }
        if toolkit_menu_enabled
            && toolkit_menus.sync(&mut m, &canvas, &mut settings_anchor, &mut actions_anchor)
        {
            dirty = true;
        }
        if dirty {
            canvas.redraw();
        }
        let ready_smoke = smoke
            && (!session_fixture
                || (m.session.mode != session_state::Mode::Opening
                    && !m.selection_refreshing()
                    && paint_revision.get() > 0))
            && m.loaded
            && (m.view != View::Groups
                || m.entries
                    .iter()
                    .filter(|e| files_core::visibility::accepts(&e.name, m.hidden_visible()))
                    .all(|e| e.stat_attempted))
            && (m.view != View::Spatial || m.layout_ready);
        let feedback_snapshot = if win.shown() {
            m.operation_feedback
                .snapshot(operation_notices::context(&m))
        } else {
            None
        };
        let mutation_snapshot = if win.shown() {
            mutation_sheets.controller.snapshot(&m)
        } else {
            None
        };
        drop(m);
        let mutation_managed = mutation_sheets.sync(mutation_snapshot, &canvas);
        model.borrow_mut().toolkit_mutation_sheet_open = mutation_sheets.is_open();
        if model.borrow().toolkit_mutation_sheet != mutation_managed {
            model.borrow_mut().toolkit_mutation_sheet = mutation_managed;
            canvas.redraw();
        }
        let feedback_managed = operation_notices.sync(feedback_snapshot, &canvas);
        if model.borrow().toolkit_feedback != feedback_managed {
            model.borrow_mut().toolkit_feedback = feedback_managed;
            canvas.redraw();
        }
        if let Some(intent) = external.present {
            let hidden = !win.shown();
            if window_presentation.schedule(
                intent,
                &model,
                &mut win,
                &mut canvas,
                paint_revision.get(),
            ) {
                waiting_for_external = false;
                if hidden
                    && toolkit_fixture
                    && std::env::var_os("FILES_TEST_PRESENTATION_CLOSE_ON_MAP").is_some()
                {
                    // Exercise the real close callback at the deferred-map
                    // boundary without posting synthetic native activation.
                    win.do_callback();
                }
                if hidden && !isolate_fixture_input {
                    model.borrow_mut().native_transfer.enable(&win);
                }
                window_presentation.pump(&model, &mut win, &mut canvas, paint_revision.get());
            }
        }
        if win.was_deleted() || canvas.was_deleted() {
            eprintln!(
                "FILES_BACKEND_WIDGET_LOST phase=presentation_admission; cancelling and draining"
            );
            runtime_failed = true;
            break;
        }
        if toolkit_fixture
            && !smoke
            && let Some(path) = &capture_path
        {
            let command = path.with_extension("popup-command");
            if let Ok(text) = std::fs::read_to_string(&command) {
                let _ = std::fs::remove_file(command);
                match text.trim() {
                    "operation-notice-state" => {
                        let value = operation_notices.fixture(&model.borrow(), &canvas);
                        eprintln!("FILES_OPERATION_NOTICE_STATE {value}");
                    }
                    "operation-notice-fixture-rename" | "operation-notice-fixture-copy" => {
                        let result = operation_notices::fixture_form(
                            &mut model.borrow_mut(),
                            text.trim().ends_with("rename"),
                        );
                        eprintln!("FILES_OPERATION_NOTICE_SETUP {result:?}");
                        canvas.redraw();
                    }
                    "operation-notice-info" => {
                        model.borrow_mut().operation_message = Some(
                            "Información de la operación; el navegador conserva el foco".into(),
                        );
                    }
                    "operation-notice-replace" => {
                        model.borrow_mut().operation_message =
                            Some("Resultado actualizado; sólo hay un aviso activo".into());
                    }
                    "operation-notice-clear" => {
                        model.borrow_mut().operation_message = None;
                    }
                    "operation-notice-protect" => model.borrow_mut().action(Action::Help),
                    "operation-notice-unprotect" => {
                        model.borrow_mut().help = None;
                    }
                    "operation-notice-clock-freeze" => fltk_omarchy::anim::clock::freeze(),
                    "operation-notice-clock-resume" => fltk_omarchy::anim::clock::resume(),
                    text if text.starts_with("operation-notice-clock-step:") => {
                        if let Ok(ms) = text[28..].parse::<u64>() {
                            fltk_omarchy::anim::clock::advance(Duration::from_millis(ms.min(1000)));
                        }
                        win.redraw();
                    }
                    "file-manager-state" => {
                        let mut value = file_manager.marker(&model.borrow());
                        value["presentation"] = window_presentation.marker();
                        value["ui"] = serde_json::json!({
                            "pump_revision":pump_revision,
                            "paint_revision":paint_revision.get(),
                            "native_key_events":presentation_key_events.get(),
                            "sidebar_width":model.borrow().rail_width()
                        });
                        eprintln!("FILES_FILE_MANAGER_STATE {value}");
                    }
                    "file-manager-protect" => model.borrow_mut().action(Action::Help),
                    "file-manager-unprotect" => {
                        model.borrow_mut().help = None;
                    }
                    "file-manager-prop-next" => model.borrow_mut().step_properties(1),
                    "file-manager-prop-prev" => model.borrow_mut().step_properties(-1),
                    "file-manager-prop-close" => model.borrow_mut().close_properties(),
                    "right-hold" | "right-release" => {
                        let rect =
                            model.borrow().hits.iter().find_map(|h| {
                                matches!(h.action, Action::Entry(_)).then_some(h.rect)
                            });
                        if let Some(r) = rect {
                            fixture_dispatch.set(true);
                            toolkit_menu::fixture_right_button(
                                &win,
                                r.x + 8,
                                r.y + 8,
                                text.trim() == "right-hold",
                            );
                            fixture_dispatch.set(false);
                        }
                    }
                    "media-inspector" => {
                        let mut state = model.borrow_mut();
                        if let Some(i) = state.entries.iter().position(|e| e.name == "red.png") {
                            state.select_entry(i, false, false);
                        }
                        state.inspector = true;
                    }
                    "media-state" => {
                        let mut state = model.borrow_mut();
                        let thumbnails = state.thumbnails.snapshot();
                        eprintln!(
                            "FILES_INSPECTOR_STATE loaded={} selected={:?} inspector={} refresh={} watch_settled={}",
                            state.loaded,
                            state.selected,
                            state.inspector,
                            state.selection_refreshing(),
                            state.listing_sync.settled()
                        );
                        eprintln!(
                            "FILES_POPUP_MEDIA menu={} generation={} thumbnails={}",
                            state.context_menu.is_some(),
                            state.generation,
                            thumbnails
                        );
                    }
                    "release-redraw" => {
                        fixture_dispatch.set(true);
                        canvas.handle_event(Event::Released);
                        fixture_dispatch.set(false);
                    }
                    "wheel-down" | "wheel-up" => {
                        let delta = if text.trim() == "wheel-down" { 3 } else { -3 };
                        crate::wheel_scroll::wheel(
                            &mut model.borrow_mut(),
                            300,
                            delta,
                            false,
                            canvas.h(),
                        );
                    }
                    "wheel-state" => {}
                    "trash-open" => model.borrow_mut().action(Action::OpenTrash),
                    "trash-close" | "trash-reload" | "trash-end" => {
                        let id = model.borrow().trash_browser.as_ref().map(|p| p.id);
                        if let Some(id) = id {
                            let command = match text.trim() {
                                "trash-close" => trash_browser::Command::Close,
                                "trash-reload" => trash_browser::Command::Reload,
                                _ => trash_browser::Command::Select(usize::MAX),
                            };
                            model.borrow_mut().action(Action::TrashBrowser(id, command));
                        }
                    }
                    "trash-state" => {}
                    "settings" => model.borrow_mut().action(Action::Settings),
                    "tooltip-settings" => {
                        let target = model.borrow().hits.iter().find_map(|h| {
                            matches!(h.action, Action::Settings).then(|| {
                                (h.rect, h.action.accessible_name().unwrap_or("Preferencias"))
                            })
                        });
                        if let Some((r, name)) = target {
                            let probe = tooltip_probe.get_or_insert_with(|| {
                                win.begin();
                                let f = Frame::new(0, 0, 1, 1, "");
                                win.end();
                                f
                            });
                            probe.resize(r.x, r.y, r.w, r.h);
                            probe.set_tooltip(name);
                            probe.show();
                            let ok = fltk_omarchy::tooltip::show(probe);
                            let state = fltk_omarchy::tooltip::shown()
                                .map(|s| {
                                    format!(
                                        "{}@{},{},{},{}",
                                        s.text, s.rect.0, s.rect.1, s.rect.2, s.rect.3
                                    )
                                })
                                .unwrap_or_default();
                            eprintln!("FILES_TOOLTIP shown={ok} {state}");
                        }
                    }
                    "tooltip-hide" => {
                        fltk_omarchy::tooltip::hide();
                        if let Some(p) = tooltip_probe.as_mut() {
                            p.hide();
                        }
                    }
                    "actions" => model.borrow_mut().action(Action::Actions),
                    "context" => {
                        crate::context_menu::open_loaded_selection(
                            &mut model.borrow_mut(),
                            canvas.w() - 3,
                            canvas.h() - 3,
                            canvas.w(),
                            canvas.h(),
                        );
                    }
                    "escape" => {
                        fltk_omarchy::focus::press("Escape");
                    }
                    "pick" => {
                        fltk_omarchy::focus::press("Enter");
                    }
                    "end" => {
                        fltk_omarchy::focus::press("End");
                    }
                    "down" => {
                        fltk_omarchy::focus::press("Down");
                    }
                    "rename-downs" => {
                        // Down presses from the first enabled root row to
                        // Rename (the popup skips disabled rows; a submenu
                        // is one row) and Rename's top-level index.
                        if let Some(menu) = model.borrow().context_menu.as_ref()
                            && let Some(index) = menu
                                .items
                                .iter()
                                .position(|i| matches!(i.action, Some(Action::Rename)))
                            && let Some((root, downs, _)) =
                                crate::toolkit_menu::context_route(menu, index)
                        {
                            eprintln!("FILES_TOOLKIT_RENAME index={root} downs={downs}");
                        }
                    }
                    "right" => {
                        fltk_omarchy::focus::press("Right");
                    }
                    t if t.starts_with("submenu-downs:") => {
                        // Route to the first entry whose label contains the
                        // text: root row, Downs to it, Downs inside its submenu.
                        let needle = &t["submenu-downs:".len()..];
                        if let Some(menu) = model.borrow().context_menu.as_ref()
                            && let Some(index) = menu
                                .items
                                .iter()
                                .position(|i| !i.section && i.label.contains(needle))
                            && let Some((root, downs, child)) =
                                crate::toolkit_menu::context_route(menu, index)
                        {
                            eprintln!(
                                "FILES_TOOLKIT_SUBMENU flat={index} root={root} downs={downs} child_downs={}",
                                child.map_or(-1, |c| c as i64)
                            );
                        }
                    }
                    "help-key" => {
                        fixture_dispatch.set(true);
                        fltk_omarchy::focus::press("F1");
                        fixture_dispatch.set(false);
                        eprintln!("FILES_TOOLKIT_HELP open={}", model.borrow().help.is_some());
                    }
                    "help-close" => model.borrow_mut().action(Action::HelpClose),
                    "actions-key" | "settings-key" | "ctrl-j" => {
                        fixture_dispatch.set(true);
                        let key = match text.trim() {
                            "actions-key" => "F10",
                            "settings-key" => "Ctrl+,",
                            _ => "Ctrl+j",
                        };
                        fltk_omarchy::focus::press(key);
                        fixture_dispatch.set(false);
                        let state = model.borrow();
                        eprintln!(
                            "FILES_TOOLKIT_KEY key={key} settings={} actions={}",
                            state.settings, state.actions_menu
                        );
                    }
                    "cancel-rename" => model.borrow_mut().action(Action::RenameCancel),
                    "outside" => {
                        fltk_omarchy::focus::click_at(&win, 0, 0);
                    }
                    "closed" => toolkit_menus.close(),
                    "close" => model.borrow_mut().closing = true,
                    "breadcrumb-state" => {
                        let state = breadcrumb_scene::state(&model.borrow(), &canvas);
                        eprintln!("FILES_BREADCRUMB_STATE {state}");
                    }
                    "breadcrumb-pick-last" => {
                        for key in ["End", "Up", "Enter"] {
                            fltk_omarchy::focus::press(key);
                        }
                    }
                    "breadcrumb-pick-background" => {
                        for key in ["End", "Right", "End", "Enter"] {
                            fltk_omarchy::focus::press(key);
                        }
                    }
                    "breadcrumb-key-open" => {
                        fixture_dispatch.set(true);
                        fltk_omarchy::focus::press("Alt+Down");
                        fixture_dispatch.set(false);
                    }
                    t if t.starts_with("breadcrumb-") => {
                        let result = breadcrumb_scene::command(&mut model.borrow_mut(), t);
                        eprintln!("FILES_BREADCRUMB_COMMAND result={result:?}");
                    }
                    "tab-context-state" => {
                        let state = tab_context_scene::state(&model.borrow(), &canvas);
                        eprintln!("FILES_TAB_CONTEXT_STATE {state}");
                    }
                    "tab-context-key-open" | "tab-context-key-left" | "tab-context-key-right" => {
                        fixture_dispatch.set(true);
                        let key = match text.trim() {
                            "tab-context-key-left" => "Ctrl+Shift+PageUp",
                            "tab-context-key-right" => "Ctrl+Shift+PageDown",
                            _ => "Ctrl+Shift+F10",
                        };
                        fltk_omarchy::focus::press(key);
                        fixture_dispatch.set(false);
                    }
                    "tab-context-move-left"
                    | "tab-context-close-right"
                    | "tab-context-close-others" => {
                        let downs = match text.trim() {
                            "tab-context-move-left" => {
                                let m = model.borrow();
                                2 + usize::from(
                                    m.tab_context
                                        .as_ref()
                                        .and_then(|t| m.tabs.index(t.id))
                                        .is_some_and(|i| i + 1 < m.tabs.rows.len()),
                                )
                            }
                            "tab-context-close-right" => 2,
                            _ => 1,
                        };
                        fltk_omarchy::focus::press("Home");
                        for _ in 0..downs {
                            fltk_omarchy::focus::press("Down");
                        }
                        fltk_omarchy::focus::press("Enter");
                    }
                    t if t.starts_with("tab-context-") => {
                        let result = tab_context_scene::command(&mut model.borrow_mut(), t);
                        eprintln!("FILES_TAB_CONTEXT_COMMAND result={result:?}");
                    }
                    "tab-widget-motion" => {
                        let result = tab_motion_scene::run(&win, &canvas, &model);
                        eprintln!("FILES_TAB_WIDGET_MOTION {result}");
                    }
                    "tab-widget-state" => {
                        let state = tab_strip.marker(&model.borrow(), &canvas);
                        eprintln!("FILES_TAB_WIDGET_STATE {state}");
                    }
                    "tab-widget-focus-browser" => {
                        let _ = canvas.take_focus();
                    }
                    t if t.starts_with("tab-widget-") => {
                        let result = tab_strip.fixture(&mut model.borrow_mut(), t);
                        eprintln!("FILES_TAB_WIDGET_COMMAND result={result:?}");
                    }
                    "tab-gesture-state" => {
                        let state = tab_gesture_scene::state(&model.borrow());
                        eprintln!("FILES_TAB_GESTURE_STATE {state}");
                    }
                    t if t.starts_with("tab-gesture:") => {
                        let result = tab_gesture_scene::command(
                            &mut model.borrow_mut(),
                            &t[12..],
                            canvas.w(),
                            canvas.h(),
                        );
                        eprintln!("FILES_TAB_GESTURE_COMMAND result={result:?}");
                    }
                    "sidebar-motion-check" => {
                        let report =
                            sidebar_motion_scene::run(&win, &canvas, &fixture_sidebar, &model);
                        eprintln!("FILES_SIDEBAR_MOTION_REPORT {report}");
                    }
                    "sidebar-expand" => fixture_sidebar.expand(),
                    "sidebar-collapse" => fixture_sidebar.collapse(),
                    "sidebar-state" => {
                        let rail = fixture_rail.borrow();
                        let keys: Vec<&str> = rail.items.iter().map(|i| i.key.as_str()).collect();
                        let state = model.borrow();
                        eprintln!(
                            "FILES_TOOLKIT_SIDEBAR expanded={} rail={} visible={} rect=({}, {}, {}, {}) active={} path={} scroll={:?} rows={:?} keys={}",
                            fixture_sidebar.is_expanded(),
                            state.rail,
                            fixture_sidebar.visible(),
                            fixture_sidebar.x(),
                            fixture_sidebar.y(),
                            fixture_sidebar.w(),
                            fixture_sidebar.h(),
                            rail.items.get(rail.active).map_or("-", |i| i.key.as_str()),
                            state.path.display(),
                            fixture_sidebar.scroll_position(),
                            (0..rail.items.len())
                                .map(|i| fixture_sidebar.item_rect(i))
                                .collect::<Vec<_>>(),
                            keys.join("|")
                        );
                    }
                    t if t.starts_with("sidebar-context:") => {
                        let key = &t["sidebar-context:".len()..];
                        let index = fixture_rail
                            .borrow()
                            .items
                            .iter()
                            .position(|i| i.key == key);
                        let rect = {
                            fixture_sidebar.tree().scroll_to(key);
                            app::flush();
                            index.and_then(|i| fixture_sidebar.item_rect(i))
                        };
                        if let Some((x, y, _, h)) = rect {
                            fltk_omarchy::focus::right_click_at(&win, x + 20, y + h / 2);
                        }
                        eprintln!("FILES_TOOLKIT_SIDEBAR_CONTEXT key={key} rect={rect:?}");
                    }
                    t if t.starts_with("sidebar-drop:") || t.starts_with("sidebar-hover:") => {
                        // The private-pipe DnD fixture (no Wayland offer) at a
                        // rail row: a uri-list naming FILES_TEST_SIDEBAR_DROP.
                        let (kind, key) = t.split_once(':').unwrap_or_default();
                        let index = fixture_rail
                            .borrow()
                            .items
                            .iter()
                            .position(|i| i.key == key);
                        let point = {
                            fixture_sidebar.tree().scroll_to(key);
                            app::flush();
                            index.and_then(|i| fixture_sidebar.item_rect(i))
                        }
                        .map(|(x, y, w, h)| (x + w / 2, y + h / 2));
                        let result = (|| -> Result<(), String> {
                            let point = point.ok_or("row not in view")?;
                            if kind == "sidebar-hover" {
                                return model.borrow_mut().fixture_drag_at(None, Some(point));
                            }
                            let source = std::path::PathBuf::from(
                                std::env::var_os("FILES_TEST_SIDEBAR_DROP")
                                    .ok_or("FILES_TEST_SIDEBAR_DROP missing")?,
                            );
                            let wire = files_core::file_transfer::encode(
                                files_core::file_transfer::Format::UriList,
                                files_core::file_transfer::Intent::Copy,
                                &[source],
                            )?;
                            let mut fds = [0; 2];
                            if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
                                return Err(std::io::Error::last_os_error().to_string());
                            }
                            use std::{io::Write, os::fd::FromRawFd};
                            let reader = unsafe { std::fs::File::from_raw_fd(fds[0]) };
                            let mut writer = unsafe { std::fs::File::from_raw_fd(fds[1]) };
                            let flags = unsafe { libc::fcntl(fds[0], libc::F_GETFL) };
                            if flags < 0
                                || unsafe {
                                    libc::fcntl(fds[0], libc::F_SETFL, flags | libc::O_NONBLOCK)
                                } < 0
                            {
                                return Err(std::io::Error::last_os_error().to_string());
                            }
                            writer.write_all(&wire).map_err(|e| e.to_string())?;
                            drop(writer);
                            model
                                .borrow_mut()
                                .fixture_drag_at(Some(reader), Some(point))
                        })();
                        let state = model.borrow();
                        eprintln!(
                            "FILES_TOOLKIT_SIDEBAR_DROP kind={kind} key={key} point={point:?} result={result:?} hover={:?}",
                            state
                                .native_transfer
                                .hover()
                                .map(|t| t.path.display().to_string())
                        );
                    }
                    "sidebar-form" => {
                        let state = model.borrow();
                        eprintln!(
                            "FILES_TOOLKIT_SIDEBAR_FORM open={} destination={} names={} message={:?}",
                            state.copy_form.is_some(),
                            state
                                .copy_form
                                .as_ref()
                                .map_or("", |d| d.destination.as_str()),
                            state.copy_form.as_ref().map_or(0, |d| d.names.len()),
                            state.operation_message
                        );
                    }
                    "sidebar-form-cancel" => model.borrow_mut().action(Action::CopyDialogCancel),
                    "sidebar-confirm" => model.borrow_mut().action(Action::CopyStart),
                    t if t.starts_with("sidebar-drag:") => {
                        // Drag row `from` onto the upper part of row `to` (drop
                        // before it), through the toolkit's mouse helpers.
                        let (from, to) = t["sidebar-drag:".len()..]
                            .split_once('|')
                            .unwrap_or_default();
                        let rect = |key: &str| {
                            let index = fixture_rail
                                .borrow()
                                .items
                                .iter()
                                .position(|i| i.key == key);
                            {
                                fixture_sidebar.tree().scroll_to(key);
                                app::flush();
                                index.and_then(|i| fixture_sidebar.item_rect(i))
                            }
                        };
                        let tree = fixture_sidebar.tree();
                        tree.reveal(to);
                        tree.reveal(from);
                        app::flush();
                        if let (Some(a), Some(b)) = (rect(from), rect(to)) {
                            let start = (a.0 + a.2 / 2, a.1 + a.3 / 2);
                            let end = (b.0 + 60.min(b.2 - 4), b.1 + 4);
                            fltk_omarchy::focus::mouse_drag(&win, start, end, false);
                            fltk_omarchy::focus::mouse_up(&win, end.0, end.1);
                        }
                        eprintln!(
                            "FILES_TOOLKIT_SIDEBAR_DRAG from={from} to={to} found={}",
                            rect(from).is_some() && rect(to).is_some()
                        );
                    }
                    "sidebar-tree" => {
                        eprintln!(
                            "FILES_TOOLKIT_TREE {}",
                            serde_json::from_str::<serde_json::Value>(
                                &fltk_omarchy::testkit::tree_json(&win)
                            )
                            .unwrap_or_default()
                        );
                    }
                    "sidebar-tabs" => {
                        let state = model.borrow();
                        eprintln!(
                            "FILES_TOOLKIT_SIDEBAR_TABS count={} path={}",
                            state.tabs.rows.len(),
                            state.path.display()
                        );
                    }
                    t if t.starts_with("sidebar-click:") || t.starts_with("sidebar-secondary:") => {
                        let (kind, key) = t.split_once(':').unwrap_or_default();
                        let index = fixture_rail
                            .borrow()
                            .items
                            .iter()
                            .position(|i| i.key == key);
                        let rect = {
                            fixture_sidebar.tree().scroll_to(key);
                            app::flush();
                            index.and_then(|i| fixture_sidebar.item_rect(i))
                        };
                        if let Some((x, y, w, h)) = rect {
                            let px = if kind == "sidebar-secondary" {
                                x + w - 18
                            } else {
                                x + 20
                            };
                            fltk_omarchy::focus::click_at(&win, px, y + h / 2);
                        }
                        eprintln!("FILES_TOOLKIT_SIDEBAR_CLICK key={key} rect={rect:?}");
                    }
                    _ => eprintln!("FILES_TOOLKIT_FIXTURE_INVALID"),
                }
                if text.trim().starts_with("trash-") {
                    let state = model.borrow();
                    eprintln!(
                        "FILES_TRASH_BROWSER open={} count={} ready={}",
                        state.trash_browser.is_some(),
                        state
                            .trash_browser
                            .as_ref()
                            .map_or(0, |p| p.catalog.items.len()),
                        state.trash_browser.as_ref().is_some_and(|p| p.ready())
                    );
                }
                if text.trim().starts_with("wheel-") {
                    eprintln!("FILES_WHEEL scroll={}", model.borrow().scroll);
                }
                eprintln!("FILES_TOOLKIT_FIXTURE action={}", text.trim());
                canvas.redraw();
            }
        }
        if ready_smoke {
            if session_fixture {
                // Observe the startup result; standard smoke intentionally mutates
                // view/filter/selection and would invalidate a real restoration.
                canvas.redraw();
                fltk_omarchy::app::flush();
                eprintln!(
                    "FILES_UI_SMOKE_OK session_observational=true entries={}",
                    model.borrow().entries.len()
                );
            } else {
                smoke_test(&model, &mut canvas);
            }
            {
                let mut state = model.borrow_mut();
                if spatial && scene.is_empty() {
                    state.action(Action::View(View::Spatial));
                }
                match scene.as_str() {
                    "du" => {
                        let index = state
                            .entries
                            .iter()
                            .position(|e| e.is_dir())
                            .expect("du scene needs a directory");
                        state.select_entry(index, false, false);
                        state.action(Action::CalculateSize);
                    }
                    "rounded" => {
                        state.action(Action::Corner(files_core::theme::Corners::Rounded));
                        state.settings = true;
                    }
                    "square" => {
                        state.action(Action::Corner(files_core::theme::Corners::Square));
                        state.settings = true;
                    }
                    "rail" => {
                        state.rail = true;
                        state.rail_request = Some(true);
                    }
                    "list" => state.action(Action::View(View::List)),
                    "spatial" | "spatial-drag" | "spatial-reopen" => {
                        state.action(Action::View(View::Spatial));
                        state.inspector = false;
                    }
                    _ => {}
                }
            }
            // smoke_test/view actions invalidate targets. Initial fixture state
            // must describe a paint after those actions, never the old frame.
            space_painted_after = paint_revision.get();
            canvas.redraw();
            smoke = false;
        }
        if space_fixture && !smoke {
            if let Some(path) = &capture_path {
                let request = path.with_extension("command");
                if let Ok(command) = std::fs::read_to_string(&request) {
                    let _ = std::fs::remove_file(request);
                    let before_paint = paint_revision.get();
                    let result = if command.trim() == "callback" {
                        win.do_callback();
                        Ok(())
                    } else if thumbnail_fixture
                        && command.trim() == "fixture-destroy-window"
                        && std::env::var_os("FILES_TEST_ALLOW_WIDGET_DELETE").is_some()
                    {
                        // Own-fixture C++ destruction, not compositor/native input.
                        app::delete_widget(win.clone());
                        Ok(())
                    } else if space_fixture
                        && matches!(
                            command.trim(),
                            "fixture-narrow" | "fixture-wide" | "fixture-short" | "fixture-minimum"
                        )
                    {
                        let (w, h) = if command.trim() == "fixture-minimum" {
                            (440, 440)
                        } else if command.trim() == "fixture-short" {
                            (800, 440)
                        } else if command.trim() == "fixture-narrow" {
                            (800, 500)
                        } else {
                            (1000, 560)
                        };
                        win.size_range(w, h, w, h);
                        win.resize(win.x(), win.y(), w, h);
                        Ok(())
                    } else if hidden_fixture
                        && matches!(
                            command.trim(),
                            "clip-text-request" | "clip-text-request-later"
                        )
                    {
                        // Own-buffer synchronous/reentrant callback contract only.
                        // No compositor focus/selection ownership is asserted.
                        if command.trim() == "clip-text-request" {
                            app::copy("texto á\tb");
                        }
                        eprintln!(
                            "FILES_TEXT_PREFLIGHT rust_wrapper={} canonical={}",
                            app::clipboard_contains(app::ClipboardContent::Text),
                            clipboard_ui::text_available()
                        );
                        clipboard_ui::request_paste(&model, &canvas);
                        Ok(())
                    } else if hidden_fixture
                        && command.trim().starts_with("tab-")
                        && std::env::var_os("FILES_TEST_TABS").is_some()
                    {
                        tabs_scene::command(
                            &mut model.borrow_mut(),
                            &space_fixture_root,
                            command.trim(),
                        )
                    } else if hidden_fixture {
                        hidden_scene::command(
                            &mut model.borrow_mut(),
                            command.trim(),
                            canvas.w(),
                            canvas.h(),
                        )
                    } else {
                        space_map_scene::command(
                            &mut model.borrow_mut(),
                            command.trim(),
                            &space_fixture_root,
                            canvas.w(),
                            canvas.h(),
                        )
                    };
                    match result {
                        Ok(()) => {
                            space_seq += 1;
                            space_marker.clear();
                            space_painted_after = before_paint;
                            eprintln!(
                                "FILES_SPACE_COMMAND seq={space_seq} action={}",
                                command.trim()
                            );
                        }
                        Err(e) => eprintln!(
                            "FILES_SPACE_COMMAND_ERROR action={} error={e}",
                            command.trim()
                        ),
                    }
                    if win.was_deleted() || canvas.was_deleted() {
                        eprintln!(
                            "FILES_BACKEND_WIDGET_LOST phase=fixture_command; cancelling and draining without widget access"
                        );
                        runtime_failed = true;
                        break;
                    }
                    canvas.redraw();
                    fltk_omarchy::app::flush();
                    if win.was_deleted() || canvas.was_deleted() {
                        eprintln!(
                            "FILES_BACKEND_WIDGET_LOST phase=after_fixture_flush; cancelling and draining without widget access"
                        );
                        runtime_failed = true;
                        break;
                    }
                }
            }
            let state = model.borrow();
            if !state.closing
                && state.loaded
                && paint_revision.get() > space_painted_after
                && let Some(marker) = if hidden_fixture {
                    hidden_scene::marker(&state, space_seq, canvas.w(), canvas.h(), &canvas.label())
                } else {
                    space_map_scene::marker(&state, space_seq)
                }
                && marker != space_marker
            {
                eprintln!("{marker}");
                space_marker = marker;
            }
        }
        if thumbnail_fixture && !smoke {
            let mut state = model.borrow_mut();
            let thumbnail = state.thumbnails.snapshot();
            let preview = match state.preview.view() {
                text_preview_state::View::Image(_) => "image",
                text_preview_state::View::Video(_) => "video",
                text_preview_state::View::Closed => "closed",
                text_preview_state::View::Loading => "loading",
                text_preview_state::View::Error(_) => "error",
                _ => "other",
            };
            let marker = format!(
                "FILES_THUMBNAILS_STATE {}",
                serde_json::json!({"seq":space_seq,"paint_revision":paint_revision.get(),"thumbnails":thumbnail,"preview":preview,"preview_drained":state.preview.is_drained(),"media_visible":state.media_visible,"closing":state.closing,"path":state.path.to_str(),"path_raw":state.path.as_os_str().as_encoded_bytes()})
            );
            if marker != thumbnail_marker {
                eprintln!("{marker}");
                thumbnail_marker = marker;
            }
        }
        if open_fixture && !smoke {
            if let Some(path) = &capture_path {
                let request = path.with_extension("command");
                if let Ok(command) = std::fs::read_to_string(&request) {
                    let _ = std::fs::remove_file(request);
                    let before_paint = paint_revision.get();
                    let result = if command.trim() == "callback" {
                        win.do_callback();
                        Ok(())
                    } else {
                        open_scene::command(&mut model.borrow_mut(), command.trim())
                    };
                    match result {
                        Ok(()) => {
                            open_seq += 1;
                            open_reported = None;
                            open_painted_after = before_paint;
                            eprintln!(
                                "FILES_OPEN_COMMAND seq={open_seq} action={}",
                                command.trim()
                            );
                        }
                        Err(error) => eprintln!(
                            "FILES_OPEN_COMMAND_ERROR action={} error={error}",
                            command.trim()
                        ),
                    }
                    canvas.redraw();
                    fltk_omarchy::app::flush();
                }
            }
            let state = model.borrow();
            if !state.closing
                && state.loaded
                && state.open_job.is_none()
                && paint_revision.get() > open_painted_after
                && open_reported != Some(open_seq)
            {
                eprintln!(
                    "FILES_OPEN_READY seq={open_seq} path={:?} error={:?}",
                    state.path, state.error
                );
                open_reported = Some(open_seq);
            }
        }
        if properties_fixture && !smoke {
            if let Some(path) = &capture_path {
                let request = path.with_extension("command");
                if let Ok(command) = std::fs::read_to_string(&request) {
                    let _ = std::fs::remove_file(request);
                    {
                        let mut state = model.borrow_mut();
                        let before_paint = paint_revision.get();
                        match properties_scene::command(&mut state, command.trim()) {
                            Ok(()) => {
                                properties_seq += 1;
                                properties_painted_after = before_paint;
                                properties_reported = None;
                                eprintln!(
                                    "FILES_PROPERTIES_COMMAND seq={properties_seq} action={} id={} applied={}",
                                    command.trim(),
                                    state.mutation_id,
                                    state.mutation_applied
                                );
                            }
                            Err(error) => eprintln!(
                                "FILES_PROPERTIES_COMMAND_ERROR action={} error={error}",
                                command.trim()
                            ),
                        }
                    }
                    canvas.redraw();
                    fltk_omarchy::app::flush();
                }
            }
            let state = model.borrow();
            if !state.closing
                && !state.mutations.busy
                && !state.mutation_applied
                && state.loaded
                && state.properties_dialog.as_ref().is_none_or(|d| !d.pending)
                && paint_revision.get() > properties_painted_after
                && properties_reported != Some(properties_seq)
            {
                if let Some(d) = &state.properties_dialog {
                    eprintln!(
                        "FILES_PROPERTIES_READY seq={properties_seq} id={} dialog=true observed={:04o} requested={:04o} stale={} editable={} message={:?}",
                        d.id,
                        d.observed.map_or(0, |p| p.mode),
                        d.requested,
                        d.stale,
                        d.editable(),
                        d.message
                    );
                } else {
                    eprintln!(
                        "FILES_PROPERTIES_READY seq={properties_seq} id={} dialog=false",
                        state.mutation_id
                    );
                }
                properties_reported = Some(properties_seq);
            }
        }
        if trash_fixture && !smoke {
            if let Some(path) = &capture_path {
                let request = path.with_extension("command");
                if let Ok(command) = std::fs::read_to_string(&request) {
                    let _ = std::fs::remove_file(request);
                    {
                        let mut state = model.borrow_mut();
                        match trash_scene::command(&mut state, command.trim()) {
                            Ok(()) => {
                                trash_seq += 1;
                                trash_reported = None;
                                eprintln!(
                                    "FILES_TRASH_COMMAND seq={trash_seq} action={} id={} applied={} undo={} notice={}",
                                    command.trim(),
                                    state.mutation_id,
                                    state.mutation_applied,
                                    state.can_undo,
                                    state.notice_open
                                );
                            }
                            Err(error) => eprintln!("FILES_TRASH_COMMAND_ERROR {error}"),
                        }
                    }
                    canvas.redraw();
                    fltk_omarchy::app::flush();
                }
            }
            let state = model.borrow();
            if !state.closing
                && !state.mutations.busy
                && !state.mutation_applied
                && state.loaded
                && trash_reported != Some(trash_seq)
            {
                eprintln!(
                    "FILES_TRASH_READY seq={trash_seq} id={} entries={} undo={} notice={} message={:?}",
                    state.mutation_id,
                    state.entries.len(),
                    state.can_undo,
                    state.notice_open,
                    state.operation_message
                );
                trash_reported = Some(trash_seq);
            }
        }
        if window_fixture && !smoke {
            if let Some(path) = &capture_path {
                let command = path.with_extension("command");
                if let Ok(text) = std::fs::read_to_string(&command) {
                    let _ = std::fs::remove_file(command);
                    let mut state = model.borrow_mut();
                    match window_scene::command(&mut state, text.trim()) {
                        Ok(()) => {
                            window_seq += 1;
                            window_reported = None;
                            eprintln!(
                                "FILES_WINDOW_COMMAND seq={window_seq} action={}",
                                text.trim()
                            );
                        }
                        Err(error) => eprintln!("FILES_WINDOW_ERROR {error}"),
                    }
                    canvas.redraw();
                }
            }
            let state = model.borrow();
            if !state.closing
                && window_reported != Some(window_seq)
                && state.window_mode == applied_window_mode
                && !state.config_dirty
                && state.config_in_flight.is_none()
            {
                // Paint synchronously before observing hits; no native input.
                drop(state);
                canvas.redraw();
                fltk_omarchy::app::flush();
                let state = model.borrow();
                window_scene::validate(&state, canvas.w(), canvas.h());
                eprintln!(
                    "FILES_WINDOW_READY seq={window_seq} mode={} preferences={} actions={} border={} fullscreen={} maximized={} width={} height={} config_error={:?}",
                    state.window_mode.key(),
                    state.settings,
                    state.actions_menu,
                    win.border(),
                    win.fullscreen_active(),
                    win.maximize_active(),
                    canvas.w(),
                    canvas.h(),
                    state.config_error
                );
                window_reported = Some(window_seq);
            }
        }
        if !smoke && scene == "du" && !du_probe_done {
            let state = model.borrow();
            if state.du_job.is_none() && state.du_target.is_some() {
                let text = state.du_text.as_deref().unwrap_or_default();
                assert!(text.starts_with("Aparente:"), "du scene failed: {text}");
                eprintln!("FILES_DU_OK {text}");
                du_probe_done = true;
            }
        }
        if !smoke
            && !spatial_probe_done
            && matches!(scene.as_str(), "spatial-drag" | "spatial-reopen")
        {
            fltk_omarchy::app::flush();
            let mut state = model.borrow_mut();
            if state.layout_ready
                && (scene == "spatial-reopen"
                    || state.order.first().is_some_and(|first| {
                        state
                            .hits
                            .iter()
                            .any(|h| matches!(h.action, Action::Entry(i) if i == *first))
                    }))
            {
                let first = state
                    .order
                    .first()
                    .copied()
                    .expect("spatial fixture is not empty");
                if scene == "spatial-drag" {
                    let target = state
                        .hits
                        .iter()
                        .find(|h| matches!(h.action, Action::Entry(i) if i == first))
                        .expect("visible spatial fixture target")
                        .rect;
                    state.pointer_down(target.x + 8, target.y + 8, false, false, false);
                    state.pointer_drag(target.x + 80, target.y + 56);
                    state.pointer_up();
                    state.action(Action::Zoom(1.15));
                    let position = state
                        .layout
                        .position(&state.entries[first].name)
                        .expect("dragged position");
                    assert!((position.x - 72.0).abs() < 0.01 && (position.y - 48.0).abs() < 0.01);
                    eprintln!(
                        "FILES_SPATIAL_DRAG_OK x={} y={} zoom={}",
                        position.x, position.y, state.layout.zoom
                    );
                } else {
                    let position = state
                        .layout
                        .position(&state.entries[first].name)
                        .expect("reopened saved position");
                    assert!((position.x - 72.0).abs() < 0.01 && (position.y - 48.0).abs() < 0.01);
                    assert!((state.layout.zoom - 1.15).abs() < 0.001);
                    eprintln!(
                        "FILES_SPATIAL_REOPEN_OK x={} y={} zoom={}",
                        position.x, position.y, state.layout.zoom
                    );
                }
                spatial_probe_done = true;
                canvas.redraw();
            }
        }
        if copy_fixture && !smoke {
            if let (Some(path), Some(root)) = (&capture_path, &copy_fixture_root) {
                let request = path.with_extension("command");
                if let Ok(command) = std::fs::read_to_string(&request) {
                    let _ = std::fs::remove_file(&request);
                    if let Some(size) = command.trim().strip_prefix("move-resize-")
                        && let Some((width, height)) = size.split_once('x')
                    {
                        let width = width.parse::<i32>().unwrap_or(1000).clamp(440, 1600);
                        let height = height.parse::<i32>().unwrap_or(650).clamp(440, 1200);
                        win.size_range(width, height, width, height);
                        win.resize(win.x(), win.y(), width, height);
                    }
                    let result = if command.trim() == "callback" {
                        win.do_callback();
                        Ok(())
                    } else {
                        copy_scene::command(&mut model.borrow_mut(), root, command.trim())
                    };
                    match result {
                        Ok(()) => {
                            move_seq += 1;
                            eprintln!("FILES_COPY_COMMAND_OK {}", command.trim());
                        }
                        Err(error) => {
                            eprintln!("FILES_COPY_COMMAND_ERROR {}: {error}", command.trim())
                        }
                    }
                    canvas.redraw();
                }
            }
            let m = model.borrow();
            let marker = format!(
                "FILES_COPY_STATE phase={:?} active={:?} queued={} loaded={} entries={} snapshot={} notice={}",
                m.copy.phase(),
                m.copy.active_id(),
                m.copy.queued_count(),
                m.loaded,
                m.entries.len(),
                m.refresh_selection.is_some(),
                m.notice_open
            );
            if marker != copy_marker {
                eprintln!("{marker}");
                copy_marker = marker;
            }
        }
        if copy_fixture && !smoke && std::env::var_os("FILES_TEST_MOVE").is_some() {
            let m = model.borrow();
            if m.loaded && !m.selection_refreshing() {
                let mut marker = serde_json::json!({"seq":move_seq,"phase":format!("{:?}",m.copy.phase()),"active":m.copy.active_id(),"queued":m.copy.queued_count(),"path":m.path.to_str(),"generation":m.generation,"selection":m.selection.iter().filter_map(|i|m.entries.get(*i)).map(|e|e.name.as_encoded_bytes().iter().map(|b|format!("{b:02x}")).collect::<String>()).collect::<Vec<_>>(),"entries":m.entries.iter().map(|e|e.name.as_encoded_bytes().iter().map(|b|format!("{b:02x}")).collect::<String>()).collect::<Vec<_>>()});
                marker["form"]=serde_json::json!(m.copy_form.as_ref().map(|d|serde_json::json!({"moving":d.moving,"extracting":d.extracting,"creating":d.creating,"preparing":d.preparing,"prepared":d.prepared.is_some()||d.archive.is_some(),"id":d.prepare_id,"error":d.error,"exdev":d.prepared.as_ref().is_some_and(|p|p.context().is_some())})));
                marker["archive_apply_painted"] = serde_json::json!(m.copy_form.as_ref().filter(|d|d.extracting).is_some_and(|d|m.hits.iter().any(|h|matches!(h.action,Action::ApplyArchive(epoch,id) if epoch==d.editor.epoch && id==d.prepare_id))));
                marker["window"] = serde_json::json!([win.w(), win.h()]);
                marker["canvas"] = serde_json::json!([canvas.w(), canvas.h()]);
                marker["painted"] = serde_json::json!([painted_size.get().0, painted_size.get().1]);
                marker["paint_revision"] = serde_json::json!(paint_revision.get());
                marker["review_pending"] =
                    serde_json::json!(m.move_preparing.is_some() || m.archive_preparing.is_some());
                marker["notice"] = serde_json::json!(m.notice_open);
                marker["outcome"]=serde_json::json!(m.copy_last.as_ref().map(|o|serde_json::json!({"id":o.id,"moving":o.moving,"extracting":o.extracting,"creating":o.creating,"archive_action":o.archive_action.map(|a|format!("{a:?}")),"success":o.result.is_ok(),"cancelled":o.cancelled(),"completed":o.report().map_or(0,|r|r.completed_count),"links":o.report().map_or(0,|r|r.symlinks_copied),"exdev":o.exdev.is_some(),"phase":o.exdev_error().and_then(|e|e.phase).map(|p|format!("{p:?}"))})));
                marker["archive_recovery"] = serde_json::json!({"actions":m.archive_recovery_actions(),"confirmation":m.archive_confirmation.map(|c|serde_json::json!({"stamp":c.stamp,"outcome":c.outcome,"action":format!("{:?}",c.action),"paused":c.paused})),"hits":m.hits.iter().filter(|h|matches!(h.action,Action::ArchiveReview(..)|Action::ArchiveCommit(_)|Action::ArchivePause(_)|Action::ArchiveBack(_)|Action::ArchiveClearPending(_))).count()});
                marker["recovery"] = serde_json::json!({"open":m.recovery_catalog.open,"loading":m.recovery_catalog.pending.is_some(),"entries":m.recovery_catalog.entries.len(),"error":m.recovery_catalog.error,"actions":m.recovery_actions()});
                let marker = format!("FILES_MOVE_STATE {marker}");
                if marker != move_marker {
                    eprintln!("{marker}");
                    move_marker = marker;
                }
            }
        }
        if mutation_fixture && !smoke {
            if let Some(path) = &capture_path {
                let request = path.with_extension("command");
                if let Ok(command) = std::fs::read_to_string(&request) {
                    let _ = std::fs::remove_file(&request);
                    // Geometry may lay out native children. Keep that work
                    // outside Model even in private diagnostics.
                    let native_marker = (command.trim() == "mutation-sheet-state")
                        .then(|| mutation_sheets.marker());
                    let sidebar_marker=(command.trim()=="usability-state").then(|| {
                        let rail=fixture_rail_state.borrow();
                        serde_json::json!({"pinned":sidebar.is_pinned(),"expanded":sidebar.is_expanded(),
                            "width":sidebar.w(),"rows":rail.items.iter().enumerate().map(|(i,row)|serde_json::json!({
                                "key":row.key,"label":row.label,"header":row.header,"footer":row.footer,
                                "rect":sidebar.item_rect(i).map(|r|[r.0,r.1,r.2,r.3]),
                            })).collect::<Vec<_>>()})
                    });
                    let mut state = model.borrow_mut();
                    match if let Some(sidebar) = sidebar_marker {
                        let space = match state.space_map.view() {
                            space_map_state::View::Closed => serde_json::json!({"state":"closed"}),
                            space_map_state::View::Loading => {
                                serde_json::json!({"state":"loading"})
                            }
                            space_map_state::View::Error(error) => {
                                serde_json::json!({"state":"error","error":error})
                            }
                            space_map_state::View::Ready(report) => {
                                serde_json::json!({"state":"ready","complete":report.complete,
                                "total":report.total_bytes,"observed":report.observed_bytes,"nodes":report.nodes.len()})
                            }
                        };
                        let thumbnail_snapshot = state.thumbnails.snapshot();
                        let marker = serde_json::json!({"thumbnails":thumbnail_snapshot,"show_thumbnails":state.show_thumbnails,"space":space,"title":win.label(),"app_id":"files",
                            "path":state.path.as_os_str().as_encoded_bytes(),"loaded":state.loaded,
                            "toolbar":toolbar::marker(&state,canvas.w(),canvas.h()),"sidebar":sidebar,
                            "scroll":state.scroll,"size_busy":state.sizes.has_work(),"size_dispatch_total":state.sizes.dispatch_count_total(),"auto_sizes":state.auto_folder_sizes,"config_pending":state.config_dirty||state.config_in_flight.is_some(),
                            "footer_status":state.footer_status,"cache_error":state.sizes.automatic_cache_error(),"cache_hits":state.sizes.automatic_cache_hits(),
                            "folders":state.visible.iter().take(64).filter_map(|&i|state.entries.get(i)).filter(|e|e.is_dir()).map(|e|serde_json::json!({
                                "name":e.name.as_encoded_bytes(),"size":state.sizes.label(&state.path,&e.name),"dispatch_count":state.sizes.dispatch_count(&state.path.join(&e.name)),
                            })).collect::<Vec<_>>(),"error":state.error});
                        eprintln!("FILES_USABILITY_STATE {marker}");
                        Ok(())
                    } else if let Some(mut marker) = native_marker {
                        marker["model"] = serde_json::json!({
                            "mode":state.rename_dialog.as_ref().map(|d| match d.mode {rename_dialog::Mode::File=>"file",rename_dialog::Mode::Folder=>"folder",rename_dialog::Mode::Rename=>"rename"}),
                            "dialog_id":state.rename_dialog.as_ref().map(|d|d.id),
                            "mutation_id":state.mutation_id,"mutation_busy":state.mutations.busy,
                            "mutation_applied":state.mutation_applied,"can_undo":state.can_undo,
                            "notice_open":state.notice_open,"closing":state.closing,
                            "legacy_canvas_dialog_enabled":!state.toolkit_mutation_sheet,
                            "browser_focus":app::focus().is_some_and(|w|w.as_widget_ptr()==canvas.as_widget_ptr())
                        });
                        eprintln!("FILES_MUTATION_SHEET_STATE {marker}");
                        Ok(())
                    } else if command.trim() == "mutation-sheet-cover" {
                        state.action(Action::Help);
                        Ok(())
                    } else {
                        mutation_scene::command(&mut state, command.trim())
                    } {
                        Ok(()) => {
                            eprintln!("FILES_MUTATION_COMMAND_OK {}", command.trim());
                            if state.mutation_applied {
                                eprintln!(
                                    "FILES_MUTATION_REQUEST id={} command={}",
                                    state.mutation_id,
                                    command.trim()
                                );
                            }
                        }
                        Err(error) => {
                            eprintln!("FILES_MUTATION_COMMAND_ERROR {}: {error}", command.trim())
                        }
                    }
                    canvas.redraw();
                }
            }
            let state = model.borrow();
            if let Some(dialog) = &state.rename_dialog {
                let marker = format!(
                    "FILES_RENAME_STATE id={} regex={} ready={} changes={} idle={} error={:?}",
                    dialog.id,
                    dialog.regex,
                    dialog.prepared.is_some(),
                    dialog.prepared.as_ref().map_or(0, |p| p.len()),
                    !state.mutations.busy && !dialog.changed && !state.mutation_applied,
                    dialog.error
                );
                if marker != rename_status_emitted {
                    eprintln!("{marker}");
                    rename_status_emitted = marker;
                }
            }
            if let Some(dialog) = &state.rename_dialog
                && let Some(prepared) = &dialog.prepared
                && rename_preview_emitted != Some(dialog.id)
            {
                eprintln!(
                    "FILES_RENAME_PREVIEW_READY count={} id={}",
                    prepared.len(),
                    dialog.id
                );
                rename_preview_emitted = Some(dialog.id);
            }
        }
        if preview_fixture && !smoke {
            if let Some(path) = &capture_path {
                let request = path.with_extension("command");
                if let Ok(command) = std::fs::read_to_string(&request) {
                    let _ = std::fs::remove_file(&request);
                    let (result, revision) = {
                        if command.trim() == "callback" {
                            win.do_callback();
                            (Ok(()), model.borrow().preview.revision())
                        } else {
                            let mut state = model.borrow_mut();
                            let result = preview_scene::command(&mut state, command.trim());
                            (result, state.preview.revision())
                        }
                    };
                    match result {
                        Ok(()) => {
                            eprintln!("FILES_PREVIEW_COMMAND_OK {}", command.trim());
                            eprintln!(
                                "FILES_PREVIEW_COMMAND_REV action={} revision={}",
                                command.trim(),
                                revision
                            );
                        }
                        Err(error) => {
                            eprintln!("FILES_PREVIEW_COMMAND_ERROR {}: {error}", command.trim())
                        }
                    }
                    canvas.redraw();
                }
            }
            let state = model.borrow();
            let marker = match state.preview.view() {
                text_preview_state::View::Ready(ready) => format!(
                    "FILES_PREVIEW_READY path={:?} encoding={} truncated={} bytes={} revision={} stamp={:?} audio_requested={} audio_running={} position={:.1} drained={} audio_error={:?}",
                    state.preview.path(),
                    ready.encoding,
                    ready.truncated,
                    ready.document.text().len(),
                    state.preview.revision(),
                    ready.source,
                    state.preview.audio_playing(),
                    state.preview.audio_running(),
                    state.preview.audio_position(),
                    state.preview.is_drained(),
                    state.preview.audio_notice()
                ),
                text_preview_state::View::Markdown(ready) => format!(
                    "FILES_PREVIEW_MARKDOWN_READY path={:?} revision={} encoding={} truncated={} bytes={} source_extent={} layout_ready={} width={} height={} step_max_us={} images_loaded={} checked_resources={} checked_root={} image_bytes={} image_buffer_bytes={} layout_error={:?}",
                    state.preview.path(),
                    state.preview.revision(),
                    ready.encoding,
                    ready.truncated,
                    ready.rendered.document.text().len(),
                    ready.rendered.source_bytes,
                    ready.rendered.layout_ready(),
                    ready.rendered.viewport.body.w,
                    ready.rendered.viewport.body.h,
                    ready.rendered.max_layout_step_us,
                    ready.rendered.loaded_images,
                    ready.resources.len(),
                    ready.source.is_some(),
                    ready.rendered.image_bytes,
                    ready.rendered.image_buffer_capacity(),
                    ready.rendered.layout_error
                ),
                text_preview_state::View::PdfText(ready) => format!(
                    "FILES_PREVIEW_PDF_TEXT_READY path={:?} revision={} page={} pages={:?} bytes={} stamp={:?}",
                    state.preview.path(),
                    state.preview.revision(),
                    state.preview.pdf_page(),
                    state.preview.pdf_pages(),
                    ready.document.text().len(),
                    ready.source
                ),
                text_preview_state::View::Pdf(ready) => format!(
                    "FILES_PREVIEW_PDF_READY path={:?} revision={} page={} pages={} width={} height={} zoom={} source={:?}",
                    state.preview.path(),
                    state.preview.revision(),
                    ready.frame.page,
                    ready.frame.pages,
                    ready.frame.bitmap.width,
                    ready.frame.bitmap.height,
                    ready.viewport.zoom.max(1.0),
                    ready.frame.bitmap.source
                ),
                text_preview_state::View::Video(ready) => format!(
                    "FILES_PREVIEW_VIDEO_READY path={:?} revision={} width={} height={} source={}x{} duration={:.6} position={:.6} codec={} playing={} sequence={:?} drained={} video_index={} audio_index={:?} audio_enabled={} notice={:?} muted={} audio_failed={} target={:?} device_scale={:.2} rate={:?}",
                    state.preview.path(),
                    state.preview.revision(),
                    ready.frame.bitmap.width,
                    ready.frame.bitmap.height,
                    ready.frame.bitmap.source_width,
                    ready.frame.bitmap.source_height,
                    ready.frame.duration,
                    ready.frame.position,
                    ready.frame.codec,
                    state.preview.video_playing(),
                    ready.sequence,
                    state.preview.is_drained(),
                    ready.frame.video_index,
                    ready.frame.audio.as_ref().map(|a| a.index),
                    state.preview.video_audio(),
                    ready.notice,
                    state.preview.muted(),
                    state.preview.video_audio_failed(),
                    state.preview.video_target(),
                    painter::device_scale(),
                    ready.frame.frame_rate
                ),
                text_preview_state::View::Html(ready) => format!(
                    "FILES_PREVIEW_HTML_READY path={:?} revision={} width={} height={} bytes={} commands={} document_height={} top={} source={:?} summary={}",
                    state.preview.path(),
                    state.preview.revision(),
                    ready.width,
                    ready.height,
                    ready.rendered.document.text().len(),
                    ready.rendered.commands().len(),
                    ready.rendered.size().1,
                    ready.rendered.top(),
                    ready.source,
                    ready.rendered.summary()
                ),
                text_preview_state::View::Image(ready) => format!(
                    "FILES_PREVIEW_IMAGE_READY width={} height={} source={}x{} format={} zoom={} path={:?} revision={} stamp={:?}",
                    ready.bitmap.width,
                    ready.bitmap.height,
                    ready.bitmap.source_width,
                    ready.bitmap.source_height,
                    ready.bitmap.format,
                    ready.viewport.zoom.max(1.0),
                    state.preview.path(),
                    state.preview.revision(),
                    ready.bitmap.source
                ),
                text_preview_state::View::Error(error) => format!(
                    "FILES_PREVIEW_ERROR path={:?} message={error}",
                    state.preview.path()
                ),
                _ => String::new(),
            };
            if marker != preview_emitted {
                if !marker.is_empty() {
                    eprintln!("{marker}");
                }
                preview_emitted = marker;
            }
        }
        if let Some(path) = &capture_path {
            let request = path.with_extension("request");
            if request.exists() {
                let capture = capture_frame::request(&request);
                let _ = std::fs::remove_file(&request);
                let capture = match capture {
                    Ok(request) => request,
                    Err(error) => {
                        eprintln!("FILES_CAPTURE_ERROR={error}");
                        continue;
                    }
                };
                let before_paint = paint_revision.get();
                if capture.redraw && !win.was_deleted() && !canvas.was_deleted() {
                    canvas.redraw();
                }
                if !capture.passive {
                    fltk_omarchy::app::flush();
                }
                if win.was_deleted() || canvas.was_deleted() {
                    eprintln!(
                        "FILES_BACKEND_WIDGET_LOST phase=capture_flush; cancelling and draining without widget access"
                    );
                    runtime_failed = true;
                    break;
                }
                win.make_current();
                match fltk::draw::capture_window(&mut win) {
                    Ok(image) => {
                        let result = capture_frame::publish(
                            path,
                            image.data_w(),
                            image.data_h(),
                            &image.to_rgb_data(),
                        );
                        if let Err(e) = result {
                            eprintln!("FILES_CAPTURE_ERROR={e}");
                        } else {
                            eprintln!(
                                "FILES_CAPTURE_OK id={} redraw={} before={} paint={} width={} height={}",
                                capture.id,
                                capture.redraw,
                                before_paint,
                                paint_revision.get(),
                                image.data_w(),
                                image.data_h()
                            );
                        }
                    }
                    Err(e) => eprintln!("FILES_CAPTURE_ERROR={e}"),
                }
            }
        }
    }
    file_manager.stop();
    mutation_sheets.close();
    operation_notices.close();
    window_presentation.close();
    if toolkit_fixture {
        eprintln!(
            "FILES_WINDOW_PRESENTATION_FINAL {}",
            window_presentation.marker()
        );
    }
    if runtime_failed {
        // A real display/poll failure must not terminate an in-flight rollback.
        // No replacement jobs or UI waits are started while draining.
        loop {
            let mut m = model.borrow_mut();
            m.closing = true;
            file_manager.pump(&mut m);
            m.mutations.cancel();
            m.copy.begin_close();
            if m.preview.visible() {
                m.preview.close();
            }
            if let Some(job) = &m.du_job {
                job.cancel();
            }
            m.pump();
            if m.mutations_drained()
                && m.copy.is_closed()
                && m.preview.exit_allowed()
                && m.thumbnails.close_allowed()
                && m.du_job.is_none()
                && m.search.is_drained()
                && !m.search_transfer.busy()
                && m.space_map.is_drained()
                && file_manager.drained()
            {
                let failure_marker = if m.preview.is_drained() {
                    "FILES_FAILURE_DRAINED"
                } else {
                    "FILES_FAILURE_EXIT_ALLOWED"
                };
                eprintln!(
                    "{failure_marker} mutations={} copy={} preview={} thumbnails={} readonly_pending={} du={} space={} preview_exit_allowed={} preview_readonly_pending={}",
                    m.mutations_drained(),
                    m.copy.is_closed(),
                    m.preview.is_drained(),
                    m.thumbnails.close_allowed(),
                    m.thumbnails.readonly_pending(),
                    m.du_job.is_none(),
                    m.space_map.is_drained(),
                    m.preview.exit_allowed(),
                    m.preview.readonly_pending()
                );
                if let Some(message) = &m.operation_message {
                    eprintln!("{message}");
                }
                for detail in &m.operation_details {
                    eprintln!("{detail}");
                }
                if m.copy_report_truncated
                    && let Some(outcome) = &m.copy_last
                {
                    eprintln!("{outcome:#?}");
                }
                break;
            }
            drop(m);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    toolkit_menus.close();
    model.borrow_mut().native_transfer.shutdown();
    if glass_report {
        let g = fltk_omarchy::glass::stats();
        eprintln!(
            "FILES_GLASS_STATS glass_fills={} opaque_fills={} regions_cleared={} regions_restored={}",
            g.glass_fills, g.opaque_fills, g.regions_cleared, g.regions_restored
        );
    }
    !runtime_failed
}
fn smoke_test(model: &Rc<RefCell<Model>>, canvas: &mut Frame) {
    let mut m = model.borrow_mut();
    let initial = m.path.clone();
    let n = m.entries.len();
    m.action(Action::View(View::List));
    assert!(m.view == View::List);
    m.query = "zz-no-match-unique".into();
    m.rebuild();
    assert!(m.order.is_empty());
    m.query.clear();
    m.rebuild();
    assert_eq!(
        m.order.len(),
        m.entries
            .iter()
            .filter(|e| files_core::visibility::accepts(&e.name, m.hidden_visible()))
            .count()
    );
    m.action(Action::View(View::Groups));
    m.action(Action::Inspector);
    m.action(Action::Inspector);
    m.action(Action::Settings);
    assert!(m.settings);
    m.action(Action::Settings);
    m.alternate_theme = true;
    m.apply_theme();
    assert_eq!(
        m.palette.background,
        files_core::theme::Palette::parse(include_str!("../../../themes/tokyo-night.toml"))
            .unwrap()
            .background
    );
    m.alternate_theme = false;
    m.apply_theme();
    assert_eq!(m.palette.background, m.system_palette.background);
    if !m.order.is_empty() {
        m.move_selection(1);
        view::reveal_selection(&mut m, canvas.w(), canvas.h());
    }
    assert_eq!(m.path, initial);
    eprintln!("FILES_UI_SMOKE_OK entries={n}");
    canvas.redraw();
}

mod move_operation;

mod move_catalog;
