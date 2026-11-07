#include "barDeco.hpp"

#include <hyprland/src/Compositor.hpp>
#include <hyprland/src/desktop/state/FocusState.hpp>
#include <hyprland/src/desktop/state/WindowState.hpp>
#include <hyprland/src/desktop/state/LayerState.hpp>
#include <hyprland/src/desktop/state/ViewHitTester.hpp>
#include <hyprland/src/desktop/view/Window.hpp>
#include <hyprland/src/desktop/view/LayerSurface.hpp>
#include <hyprland/src/helpers/MiscFunctions.hpp>
#include <hyprland/src/managers/KeybindManager.hpp>
#include <hyprland/src/managers/SeatManager.hpp>
#include <hyprland/src/managers/input/InputManager.hpp>
#include <hyprland/src/render/Renderer.hpp>
#include <hyprland/src/config/ConfigManager.hpp>
#include <hyprland/src/config/shared/animation/AnimationTree.hpp>
#include <hyprland/src/config/shared/parserUtils/ParserUtils.hpp>
#include <hyprland/src/config/supplementary/executor/Executor.hpp>
#include <hyprland/src/config/shared/actions/ConfigActions.hpp>
#include <hyprland/src/animation/AnimationManager.hpp>
#include <hyprland/src/protocols/LayerShell.hpp>
#include <hyprland/src/event/EventBus.hpp>
#include <hyprland/src/layout/LayoutManager.hpp>
#include <hyprland/src/render/OpenGL.hpp>
#include <hyprland/src/state/MonitorState.hpp>

#include "globals.hpp"
#include "BarPassElement.hpp"
#include <hyprland/src/managers/fullscreen/FullscreenController.hpp>
#include "appName.hpp"

#include <hyprland/src/protocols/XDGShell.hpp>
// The decoration mode each toplevel asked for is only kept in the protocol's private map.
#define private public
#include <hyprland/src/protocols/XDGDecoration.hpp>
#include <hyprland/src/protocols/core/Compositor.hpp>
#include <hyprland/src/managers/eventLoop/EventLoopManager.hpp>
#include <hyprland/src/managers/eventLoop/EventLoopTimer.hpp>
#undef private
#include <hyprgraphics/image/Image.hpp>

#include <algorithm>
#include <climits>

using namespace Render::GL;

static CHyprColor configColor(Config::INTEGER color) { return CHyprColor{static_cast<uint64_t>(color)}; }

static std::string windowCommand(std::string command, const PHLWINDOW &window) {
  // Resolve the decoration owner synchronously. An asynchronously spawned
  // command must not choose its target from whatever happens to be focused.
  constexpr std::string_view token = "%WINDOW%";
  const auto address = std::format("0x{:x}", reinterpret_cast<uintptr_t>(window.get()));
  size_t offset = 0;
  while ((offset = command.find(token, offset)) != std::string::npos) {
    command.replace(offset, token.size(), address);
    offset += address.size();
  }
  return command;
}

// Whether a window draws its own titlebar. GTK and libadwaita apps never bind
// xdg-decoration and always draw one. A client asking xdg-decoration for
// client-side mode can't be told apart from one that just wants no decorations
// at all (Alacritty with decorations = "None" asks the same), so those are only
// trusted to draw their own when they carry client_decorations_tag, which
// Omarchy gives Chromium-based browsers. Any client asking for server-side
// mode, such as Chromium with "Use system title bar and borders", gets a bar.
static bool clientDrawsDecorations(const PHLWINDOW &window) {
  if (!window || window->m_isX11)
    return false;

  const auto xdg = window->m_xdgSurface.lock();
  const auto toplevel = xdg ? xdg->m_toplevel.lock() : nullptr;
  if (!toplevel || !PROTO::xdgDecoration)
    return false;

  for (const auto &[resource, decoration] : PROTO::xdgDecoration->m_decorations) {
    if (!decoration || CXDGToplevelResource::fromResource(resource) != toplevel)
      continue;

    if (decoration->mostRecentlyRequested == ZXDG_TOPLEVEL_DECORATION_V1_MODE_SERVER_SIDE)
      return false;

    const auto tag = g_pOmarchyFloatingGlobalState->config.clientDecorationsTag->value();
    return !tag.empty() && window->m_ruleApplicator->m_tagKeeper.isTagged(tag);
  }

  return true;
}

COmarchyFloatingHyprBar::COmarchyFloatingHyprBar(PHLWINDOW pWindow) : IHyprWindowDecoration(pWindow) {
  m_pWindow = pWindow;

  const auto PMONITOR = pWindow->m_monitor.lock();
  PMONITOR->m_scheduledRecalc = true;

  // button events
  m_pMouseButtonCallback =
      Event::bus()->m_events.input.mouse.button.listen([&](IPointer::SButtonEvent e, Event::SCallbackInfo &info) { onMouseButton(info, e); });
  m_pTouchDownCallback = Event::bus()->m_events.input.touch.down.listen([&](ITouch::SDownEvent e, Event::SCallbackInfo &info) { onTouchDown(info, e); });
  m_pTouchUpCallback = Event::bus()->m_events.input.touch.up.listen([&](ITouch::SUpEvent e, Event::SCallbackInfo &info) { onTouchUp(info, e); });

  // move events
  m_pTouchMoveCallback = Event::bus()->m_events.input.touch.motion.listen([&](ITouch::SMotionEvent e, Event::SCallbackInfo &info) { onTouchMove(info, e); });
  m_pMouseMoveCallback = Event::bus()->m_events.input.mouse.move.listen([&](Vector2D c, Event::SCallbackInfo &info) { onMouseMove(c); });

  // Hyprland has no minimized state and drops a client's request for one, so
  // the minimize button an app draws itself would do nothing. Hand the request
  // to the same command the bar's own minimize button runs.
  if (const auto xdg = pWindow->m_xdgSurface.lock(); xdg && xdg->m_toplevel)
    m_pMinimizeRequestCallback = xdg->m_toplevel->m_events.stateChanged.listen([this] { onStateChanged(); });

  Animation::mgr()->createAnimation(configColor(g_pOmarchyFloatingGlobalState->config.barColor->value()), m_cRealBarColor,
                                    Config::animationTree()->getAnimationPropertyConfig("border"), pWindow, AVARDAMAGE_NONE);
  m_cRealBarColor->setUpdateCallback([&](auto) { damageEntire(); });
}

// Hyprland tells apps it can maximize and fullscreen a window, but not minimize
// one, so GTK greys out the minimize button it draws. Where a minimize request has
// somewhere to go, tell the app so; elsewhere, tell it what Hyprland told it.
static void advertiseMinimize(const PHLWINDOW &window, bool minimize) {
  const auto xdg = window->m_xdgSurface.lock();
  const auto toplevel = xdg ? xdg->m_toplevel.lock() : nullptr;
  const auto surface = xdg ? xdg->m_surface.lock() : nullptr;
  if (!toplevel || !surface || !surface->getResource())
    return;

  struct SSearch {
    void *toplevel;
    CXdgToplevel *found;
  } search{toplevel.get(), nullptr};
  wl_client_for_each_resource(
      wl_resource_get_client(surface->getResource()->resource()),
      [](wl_resource *resource, void *data) {
        auto search = static_cast<SSearch *>(data);
        if (std::string_view{wl_resource_get_class(resource)} != "xdg_toplevel" || wl_resource_get_version(resource) < 5)
          return WL_ITERATOR_CONTINUE;
        const auto object = static_cast<CXdgToplevel *>(wl_resource_get_user_data(resource));
        if (!object || object->data() != search->toplevel)
          return WL_ITERATOR_CONTINUE;
        search->found = object;
        return WL_ITERATOR_STOP;
      },
      &search);
  if (!search.found)
    return;

  wl_array capabilities;
  wl_array_init(&capabilities);
  for (const auto capability : {XDG_TOPLEVEL_WM_CAPABILITIES_MAXIMIZE, XDG_TOPLEVEL_WM_CAPABILITIES_FULLSCREEN, XDG_TOPLEVEL_WM_CAPABILITIES_MINIMIZE}) {
    if (capability == XDG_TOPLEVEL_WM_CAPABILITIES_MINIMIZE && !minimize)
      continue;
    *static_cast<uint32_t *>(wl_array_add(&capabilities, sizeof(uint32_t))) = capability;
  }
  search.found->sendWmCapabilities(&capabilities);
  wl_array_release(&capabilities);
  // Capabilities take effect with the next configure.
  xdg->scheduleConfigure();
}

// A window the bar belongs to: floating, on a workspace tagged for bars.
static bool onBarWorkspace(const PHLWINDOW &window) {
  const auto tag = g_pOmarchyFloatingGlobalState->config.workspaceTag->value();
  return window->m_isFloating && (tag.empty() || window->m_ruleApplicator->m_tagKeeper.isTagged(tag));
}

void COmarchyFloatingHyprBar::onStateChanged() {
  const auto PWINDOW = m_pWindow.lock();
  const auto xdg = PWINDOW ? PWINDOW->m_xdgSurface.lock() : nullptr;
  const auto toplevel = xdg ? xdg->m_toplevel.lock() : nullptr;
  if (!toplevel)
    return;

  auto run = [&](const std::string &command) {
    if (!command.empty())
      Config::Supplementary::executor()->spawn(windowCommand(command, PWINDOW));
  };

  // Only where the bar's own minimize button would be: elsewhere nothing brings
  // the window back, and the app was never told minimizing is supported.
  if (toplevel->m_state.requestsMinimize.value_or(false) && onBarWorkspace(PWINDOW))
    run(g_pOmarchyFloatingGlobalState->config.onMinimizeRequest->value());

  // Hyprland drops maximize requests here (suppress_event), so a maximize button the
  // app draws itself would do nothing either. Hyprland also tells every window it
  // is maximized, as it draws no client margins, so apps only ever ask to be
  // restored; on a floating workspace either request toggles the window, as the
  // bar's maximize button does.
  const auto onMaximize = g_pOmarchyFloatingGlobalState->config.onMaximizeRequest->value();
  if (toplevel->m_state.requestsMaximize.has_value() && !onMaximize.empty() && onBarWorkspace(PWINDOW)) {
    run(onMaximize);

    // An app that asked to be restored waits to hear it was, and Chrome stops asking
    // anything until then. Tell it so, and once it has seen that, tell it again it
    // is maximized, as Hyprland would have it. Both in one configure would cancel
    // out, so the second waits for the app to ack the first.
    m_restoreSerial = toplevel->setMaximized(false);
    m_pRestoreAckCallback = xdg->m_events.ack.listen([this](uint32_t serial) {
      if (serial >= m_restoreSerial)
        tellMaximized();
    });
    if (!m_pTellMaximizedTimer) {
      m_pTellMaximizedTimer = makeShared<CEventLoopTimer>(std::nullopt, [this](SP<CEventLoopTimer>, void *) { tellMaximized(); }, nullptr);
      g_pEventLoopManager->addTimer(m_pTellMaximizedTimer);
    }
    m_pTellMaximizedTimer->updateTimeout(std::chrono::milliseconds(500));
  }
}

// Hyprland leaves focus on a window it hides on a special workspace, as minimizing
// does, so the app never hears it went away nor came back. Chrome takes itself for
// minimized until it is activated again, and ignores its own maximize button until
// then. Tell it it is not active while hidden, and active again once it is back.
void COmarchyFloatingHyprBar::onMovedToWorkspace(PHLWORKSPACE workspace) {
  const auto PWINDOW = m_pWindow.lock();
  const auto xdg = PWINDOW ? PWINDOW->m_xdgSurface.lock() : nullptr;
  const auto toplevel = xdg ? xdg->m_toplevel.lock() : nullptr;
  if (!toplevel || !workspace)
    return;

  if (workspace->m_isSpecialWorkspace && !workspace->isVisibleNotCovered()) {
    m_bHiddenInactive = true;
    toplevel->setActive(false);
  } else if (m_bHiddenInactive) {
    m_bHiddenInactive = false;
    toplevel->setActive(Desktop::focusState()->window() == PWINDOW);
  }
}

void COmarchyFloatingHyprBar::tellMaximized() {
  m_pRestoreAckCallback.reset();
  if (m_pTellMaximizedTimer)
    m_pTellMaximizedTimer->updateTimeout(std::nullopt);
  const auto PWINDOW = m_pWindow.lock();
  const auto xdg = PWINDOW ? PWINDOW->m_xdgSurface.lock() : nullptr;
  if (const auto toplevel = xdg ? xdg->m_toplevel.lock() : nullptr)
    toplevel->setMaximized(true);
}

COmarchyFloatingHyprBar::~COmarchyFloatingHyprBar() {
  if (m_pTellMaximizedTimer)
    g_pEventLoopManager->removeTimer(m_pTellMaximizedTimer);
  std::erase(g_pOmarchyFloatingGlobalState->bars, m_self);
}

SDecorationPositioningInfo COmarchyFloatingHyprBar::getPositioningInfo() {
  const auto HEIGHT = g_pOmarchyFloatingGlobalState->config.barHeight->value();
  const auto ENABLED = g_pOmarchyFloatingGlobalState->config.enabled->value();
  const auto PRECEDENCE = g_pOmarchyFloatingGlobalState->config.barPrecedenceOverBorder->value();

  SDecorationPositioningInfo info;
  info.policy = m_hidden ? DECORATION_POSITION_ABSOLUTE : DECORATION_POSITION_STICKY;
  info.edges = DECORATION_EDGE_TOP;
  info.priority = PRECEDENCE ? 10005 : 5000;
  info.reserved = true;
  info.desiredExtents = {{0, m_hidden || !ENABLED ? 0 : HEIGHT}, {0, 0}};
  return info;
}

void COmarchyFloatingHyprBar::onPositioningReply(const SDecorationPositioningReply &reply) {
  if (reply.assignedGeometry.size() != m_bAssignedBox.size())
    m_bWindowSizeChanged = true;

  m_bAssignedBox = reply.assignedGeometry;
}

std::string COmarchyFloatingHyprBar::getDisplayName() { return "Hyprbar"; }

bool COmarchyFloatingHyprBar::inputIsValid() {
  if (m_hidden || !g_pOmarchyFloatingGlobalState->config.enabled->value())
    return false;

  if (g_pSeatManager->m_seatGrab && !g_pSeatManager->m_seatGrab->accepts(m_pWindow->wlSurface()->resource()))
    return false;

  const auto MOUSE = g_pInputManager->getMouseCoordsInternal();
  auto PMONITOR = Desktop::focusState()->monitor();

  if (!PMONITOR)
    return false;

  Desktop::CViewHitTester hitTester{*Desktop::viewState()};

  const auto WINDOWATCURSOR = hitTester.windowAt(MOUSE, Desktop::View::RESERVED_EXTENTS | Desktop::View::INPUT_EXTENTS | Desktop::View::ALLOW_FLOATING);

  auto focusState = Desktop::focusState();
  auto window = focusState->window();

  if (WINDOWATCURSOR != m_pWindow && m_pWindow != window)
    return false;

  PHLLS foundSurface = nullptr;
  Vector2D surfaceCoords;

  // Check Top Layer
  hitTester.layerSurfaceAt(MOUSE, &PMONITOR->m_layerSurfaceLayers[ZWLR_LAYER_SHELL_V1_LAYER_TOP], &surfaceCoords, &foundSurface);
  if (foundSurface)
    return false;

  // Check Overlay Layer
  hitTester.layerSurfaceAt(MOUSE, &PMONITOR->m_layerSurfaceLayers[ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY], &surfaceCoords, &foundSurface);
  if (foundSurface)
    return false;

  return true;
}

void COmarchyFloatingHyprBar::onMouseButton(Event::SCallbackInfo &info, IPointer::SButtonEvent e) {
  if (e.button != BTN_LEFT)
    return;

  if (e.state != WL_POINTER_BUTTON_STATE_PRESSED) {
    // A titlebar drag can finish over a panel or another surface.
    if (m_bDraggingThis || m_bCancelledDown || m_bDragPending)
      handleUpEvent(info);
    return;
  }

  if (!inputIsValid())
    return;

  handleDownEvent(info, std::nullopt);
}

void COmarchyFloatingHyprBar::onTouchDown(Event::SCallbackInfo &info, ITouch::SDownEvent e) {
  // Don't do anything if you're already grabbed a window with another finger
  if (!inputIsValid() || e.touchID != 0)
    return;

  handleDownEvent(info, e);
}

void COmarchyFloatingHyprBar::onTouchUp(Event::SCallbackInfo &info, ITouch::SUpEvent e) {
  if (!m_bDragPending || !m_bTouchEv || e.touchID != m_touchId)
    return;

  handleUpEvent(info);
}

void COmarchyFloatingHyprBar::onMouseMove(Vector2D coords) {
  // Hovered buttons are highlighted, so redraw when the pointer moves between
  // them, also with hardware cursors.
  damageOnButtonHover();

  if (!m_bDragPending || m_bTouchEv || !validMapped(m_pWindow) || m_touchId != 0)
    return;

  m_bDragPending = false;
  handleMovement();
}

void COmarchyFloatingHyprBar::onTouchMove(Event::SCallbackInfo &info, ITouch::SMotionEvent e) {
  if (!m_bDragPending || !m_bTouchEv || !validMapped(m_pWindow) || e.touchID != m_touchId)
    return;

  auto PMONITOR = m_pWindow->m_monitor.lock();
  PMONITOR = PMONITOR ? PMONITOR : Desktop::focusState()->monitor();
  const auto COORDS = Vector2D(PMONITOR->m_position.x + e.pos.x * PMONITOR->m_size.x, PMONITOR->m_position.y + e.pos.y * PMONITOR->m_size.y);

  if (!m_bDraggingThis) {
    // Initial setup for dragging a window.
    g_pKeybindManager->m_dispatchers["setfloating"]("activewindow");
    g_pKeybindManager->m_dispatchers["resizewindowpixel"]("exact 50% 50%,activewindow");
    // pin it so you can change workspaces while dragging a window
    g_pKeybindManager->m_dispatchers["pin"]("activewindow");
  }
  g_pKeybindManager->m_dispatchers["movewindowpixel"](std::format("exact {} {},activewindow", (int)(COORDS.x - (assignedBoxGlobal().w / 2)), (int)COORDS.y));
  m_bDraggingThis = true;
}

void COmarchyFloatingHyprBar::handleDownEvent(Event::SCallbackInfo &info, std::optional<ITouch::SDownEvent> touchEvent) {
  m_bTouchEv = touchEvent.has_value();
  if (m_bTouchEv)
    m_touchId = touchEvent.value().touchID;

  const auto PWINDOW = m_pWindow.lock();

  auto COORDS = cursorRelativeToBar();
  if (m_bTouchEv) {
    ITouch::SDownEvent e = touchEvent.value();
    PHLMONITOR PMONITOR = nullptr;
    for (auto &m : State::monitorState()->monitors()) {
      if (m->m_name == (!e.device->m_boundOutput.empty() ? e.device->m_boundOutput : "")) {
        PMONITOR = m;
        break;
      }
    }
    PMONITOR = PMONITOR ? PMONITOR : Desktop::focusState()->monitor();
    COORDS = Vector2D(PMONITOR->m_position.x + e.pos.x * PMONITOR->m_size.x, PMONITOR->m_position.y + e.pos.y * PMONITOR->m_size.y) - assignedBoxGlobal().pos();
  }

  const auto HEIGHT = g_pOmarchyFloatingGlobalState->config.barHeight->value();
  const auto BARBUTTONPADDING = g_pOmarchyFloatingGlobalState->config.barButtonPadding->value();
  const auto BARPADDING = g_pOmarchyFloatingGlobalState->config.barPadding->value();
  const auto ALIGNBUTTONS = g_pOmarchyFloatingGlobalState->config.barButtonsAlignment->value();
  const auto ON_DOUBLE_CLICK = g_pOmarchyFloatingGlobalState->config.onDoubleClick->value();

  const bool BUTTONSRIGHT = ALIGNBUTTONS != "left";

  // Hyprland measures its border resize band from the window's content, so for
  // a titled window the band sits under the bar and the edge above the bar
  // never resizes. A press in the band above the bar starts that resize here.
  if (!m_bTouchEv && startTopEdgeResize(info, PWINDOW, COORDS))
    return;

  if (!VECINRECT(COORDS, 0, 0, assignedBoxGlobal().w, HEIGHT - 1)) {

    if (m_bDraggingThis) {
      if (m_bTouchEv)
        g_pKeybindManager->m_dispatchers["settiled"]("activewindow");
      g_pKeybindManager->m_dispatchers["mouse"]("0movewindow");
      Log::logger->log(Log::DEBUG, "[hyprbars] Dragging ended on {:x}", (uintptr_t)PWINDOW.get());
    }

    m_bDraggingThis = false;
    m_bDragPending = false;
    m_bTouchEv = false;
    return;
  }

  if (Desktop::focusState()->window() != PWINDOW)
    Desktop::focusState()->fullWindowFocus(PWINDOW, Desktop::FOCUS_REASON_CLICK);

  if (PWINDOW->m_isFloating)
    Desktop::windowState()->raise(PWINDOW);

  info.cancelled = true;
  m_bCancelledDown = true;

  if (doButtonPress(BARPADDING, BARBUTTONPADDING, HEIGHT, COORDS, BUTTONSRIGHT))
    return;

  if (!ON_DOUBLE_CLICK.empty() &&
      std::chrono::duration_cast<std::chrono::milliseconds>(Time::steadyNow() - m_lastMouseDown).count() < 400 /* Arbitrary delay I found suitable */) {
    Config::Supplementary::executor()->spawn(windowCommand(ON_DOUBLE_CLICK, PWINDOW));
    m_bDragPending = false;
  } else {
    m_lastMouseDown = Time::steadyNow();
    m_bDragPending = true;
  }
}

bool COmarchyFloatingHyprBar::startTopEdgeResize(Event::SCallbackInfo &info, PHLWINDOW window, const Vector2D &coords) {
  static const auto RESIZEONBORDER = CConfigValue<Config::INTEGER>("general:resize_on_border");
  static const auto EXTENDGRAB = CConfigValue<Config::INTEGER>("general:extend_border_grab_area");

  if (!*RESIZEONBORDER || !window || !window->m_isFloating || Fullscreen::controller()->isFullscreen(window))
    return false;

  // The band runs from the top of the bar out over the border and the grab
  // area beyond it, across the bar's width and the grab area at its corners.
  const double GRAB = window->getRealBorderSize() + *EXTENDGRAB;
  const double WIDTH = assignedBoxGlobal().w;
  if (coords.y >= 0 || coords.y < -GRAB || coords.x < -GRAB || coords.x >= WIDTH + GRAB)
    return false;

  if (Desktop::focusState()->window() != window)
    Desktop::focusState()->fullWindowFocus(window, Desktop::FOCUS_REASON_CLICK);
  Desktop::windowState()->raise(window);

  CKeybindManager::changeMouseBindMode(MBIND_RESIZE);

  // The press never reaches Hyprland, so neither does the release: the up
  // handler ends the resize the way it ends a titlebar drag.
  info.cancelled = true;
  m_bCancelledDown = true;
  m_bDraggingThis = true;
  m_bDragPending = false;
  return true;
}

void COmarchyFloatingHyprBar::handleUpEvent(Event::SCallbackInfo &info) {
  if (m_pWindow.lock() != Desktop::focusState()->window())
    return;

  if (m_bCancelledDown)
    info.cancelled = true;

  m_bCancelledDown = false;

  if (m_bDraggingThis) {
    g_pKeybindManager->changeMouseBindMode(MBIND_INVALID);
    m_bDraggingThis = false;
    if (m_bTouchEv)
      (void)Config::Actions::floatWindow(Config::Actions::eTogglableAction::TOGGLE_ACTION_DISABLE);

    Log::logger->log(Log::DEBUG, "[hyprbars] Dragging ended on {:x}", (uintptr_t)m_pWindow.lock().get());
  }

  m_bDragPending = false;
  m_bTouchEv = false;
  m_touchId = 0;
}

void COmarchyFloatingHyprBar::handleMovement() {
  const auto window = m_pWindow.lock();
  const auto cursor = g_pInputManager->getMouseCoordsInternal();
  const auto bar = assignedBoxGlobal();
  const auto anchor = std::clamp((cursor.x - bar.x) / std::max(1.0, bar.w), 0.0, 1.0);
  const auto offset = std::clamp(cursor.y - bar.y, 0.0, bar.h);
  const bool maximized = Fullscreen::controller()->isFullscreen(window, Fullscreen::FSMODE_MAXIMIZED);
  if (maximized)
    Fullscreen::controller()->setFullscreenMode(window, Fullscreen::FSMODE_NONE);

  if (maximized) {
    auto box = window->layoutTarget()->position();
    const auto reserved = window->getFullWindowReservedArea();
    box.x = cursor.x - box.w * anchor;
    box.y = cursor.y + reserved.topLeft.y - offset;
    g_layoutManager->setTargetGeom(box, window->layoutTarget());
    window->layoutTarget()->warpPositionSize();
  }
  g_pKeybindManager->changeMouseBindMode(MBIND_MOVE);
  m_bDraggingThis = true;
  Log::logger->log(Log::DEBUG, "[hyprbars] Dragging initiated on {:x}", (uintptr_t)m_pWindow.lock().get());
  return;
}

bool COmarchyFloatingHyprBar::doButtonPress(Config::INTEGER barPadding, Config::INTEGER barButtonPadding, Config::INTEGER barHeight, Vector2D COORDS,
                                            const bool BUTTONSRIGHT) {
  // check if on a button
  float offset = barPadding;

  for (auto &b : g_pOmarchyFloatingGlobalState->buttons) {
    const auto BARBUF = Vector2D{(int)assignedBoxGlobal().w, barHeight};
    Vector2D currentPos = Vector2D{(BUTTONSRIGHT ? BARBUF.x - barButtonPadding - b.size - offset : offset), (BARBUF.y - b.size) / 2.0}.floor();

    if (VECINRECT(COORDS, currentPos.x, currentPos.y, currentPos.x + b.size + barButtonPadding, currentPos.y + b.size)) {
      // hit on close
      g_pKeybindManager->m_dispatchers["exec"](windowCommand(b.cmd, m_pWindow.lock()));
      return true;
    }

    offset += barButtonPadding + b.size;
  }
  return false;
}

void COmarchyFloatingHyprBar::renderBarTitle(const Vector2D &bufferSize, const float scale, const int reservedWidth) {
  const auto COLORVAL = g_pOmarchyFloatingGlobalState->config.textColor->value();
  const auto SIZE = g_pOmarchyFloatingGlobalState->config.barTextSize->value();
  const auto WEIGHT = g_pOmarchyFloatingGlobalState->config.barTextWeight->value();
  const auto FONT = g_pOmarchyFloatingGlobalState->config.barTextFont->value();
  const auto ALIGN = g_pOmarchyFloatingGlobalState->config.barTextAlign->value();
  const auto BARPADDING = g_pOmarchyFloatingGlobalState->config.barPadding->value();
  const auto BARBUTTONPADDING = g_pOmarchyFloatingGlobalState->config.barButtonPadding->value();

  float buttonSizes = BARBUTTONPADDING;
  for (auto &b : g_pOmarchyFloatingGlobalState->buttons) {
    buttonSizes += b.size + BARBUTTONPADDING;
  }

  const int scaledSize = std::round(SIZE * scale);
  const auto scaledButtonsSize = buttonSizes * scale;
  const auto scaledBarPadding = BARPADDING * scale;
  const int paddingTotal = scaledBarPadding * 2 + scaledButtonsSize + (ALIGN != "left" ? scaledButtonsSize : 0) + reservedWidth;
  const int maxWidth = std::clamp(static_cast<int>(bufferSize.x - paddingTotal), 0, INT_MAX);

  m_pPrefixTex = nullptr;
  if (m_szLastTitle.empty() || maxWidth < 1) {
    m_pTextTex = nullptr;
    return;
  }

  const CHyprColor COLOR = m_bForcedTitleColor.value_or(configColor(COLORVAL));

  // The app's name goes first and gets at most half the room, so the title,
  // which is what changes, always keeps the rest.
  int titleWidth = maxWidth;
  if (!m_szLastPrefix.empty()) {
    m_pPrefixTex = g_pHyprRenderer->renderText(m_szLastPrefix, COLOR, scaledSize, false, FONT, maxWidth / 2, WEIGHT.m_value);
    if (m_pPrefixTex && m_pPrefixTex->m_texID != 0)
      titleWidth -= m_pPrefixTex->m_size.x + std::round(SIZE * 0.6 * scale);
    else
      m_pPrefixTex = nullptr;
  }

  m_pTextTex = titleWidth > 0 ? g_pHyprRenderer->renderText(m_szLastTitle, COLOR, scaledSize, false, FONT, titleWidth, WEIGHT.m_value) : nullptr;
}

size_t COmarchyFloatingHyprBar::getVisibleButtonCount(Config::INTEGER barButtonPadding, Config::INTEGER barPadding, const Vector2D &bufferSize,
                                                      const float scale) {
  float availableSpace = bufferSize.x - barPadding * scale * 2;
  size_t count = 0;

  for (const auto &button : g_pOmarchyFloatingGlobalState->buttons) {
    const float buttonSpace = (button.size + barButtonPadding) * scale;
    if (availableSpace >= buttonSpace) {
      count++;
      availableSpace -= buttonSpace;
    } else
      break;
  }

  return count;
}

void COmarchyFloatingHyprBar::renderBarButtons(CBox *barBox, const float scale, const float a) {
  const auto BARBUTTONPADDING = g_pOmarchyFloatingGlobalState->config.barButtonPadding->value();
  const auto BARPADDING = g_pOmarchyFloatingGlobalState->config.barPadding->value();
  const auto ALIGNBUTTONS = g_pOmarchyFloatingGlobalState->config.barButtonsAlignment->value();
  const auto INACTIVECOLOR = g_pOmarchyFloatingGlobalState->config.inactiveButtonColor->value();
  const auto BUTTONROUNDING = g_pOmarchyFloatingGlobalState->config.buttonRounding->value();
  const auto BUTTONBORDERSIZE = g_pOmarchyFloatingGlobalState->config.buttonBorderSize->value();
  const auto BUTTONBORDERCOLOR = g_pOmarchyFloatingGlobalState->config.buttonBorderColor->value();

  const bool BUTTONSRIGHT = ALIGNBUTTONS != "left";
  const auto visibleCount = getVisibleButtonCount(BARBUTTONPADDING, BARPADDING, Vector2D{barBox->w, barBox->h}, scale);
  const bool INVALIDATEICONS = m_bButtonsDirty || m_bWindowSizeChanged;

  int offset = BARPADDING * scale;
  for (size_t i = 0; i < visibleCount; ++i) {
    auto &button = g_pOmarchyFloatingGlobalState->buttons[i];
    const auto scaledButtonSize = button.size * scale;
    const auto scaledButtonsPad = BARBUTTONPADDING * scale;

    auto color = button.bgcol;
    // On a glassy bar a button's face would be a solid square on the glass, so
    // only its symbol and the hover highlight are drawn.
    const bool GLASSY = glassy();

    if (INACTIVECOLOR > 0) {
      color = m_bWindowHasFocus ? color : configColor(INACTIVECOLOR);
      if (INVALIDATEICONS && button.userfg && button.iconTex)
        button.iconTex = nullptr;
    }

    color.a *= a;

    CBox buttonBox = {barBox->x + (BUTTONSRIGHT ? barBox->w - offset - scaledButtonSize : offset), barBox->y + (barBox->h - scaledButtonSize) / 2.0,
                      scaledButtonSize, scaledButtonSize};
    buttonBox.round();

    const int maxRounding = static_cast<int>(std::round(scaledButtonSize / 2.0));
    const int rounding = BUTTONROUNDING < 0 ? maxRounding : std::clamp(static_cast<int>(std::round(BUTTONROUNDING * scale)), 0, maxRounding);
    const int borderSize = std::clamp(static_cast<int>(std::round(BUTTONBORDERSIZE * scale)), 0, maxRounding);

    if (borderSize > 0) {
      auto borderColor = configColor(BUTTONBORDERCOLOR);
      borderColor.a *= a;
      g_pHyprOpenGL->renderRect(buttonBox, borderColor, {.round = rounding});
      auto face = buttonBox.copy().expand(-borderSize);
      if (!GLASSY)
        g_pHyprOpenGL->renderRect(face, color, {.round = std::max(0, rounding - borderSize)});
    } else if (!GLASSY)
      g_pHyprOpenGL->renderRect(buttonBox, color, {.round = rounding});

    // A hovered button lifts its face in its own icon color, which follows any theme.
    if (m_iButtonHoverState & (1u << i)) {
      auto hover = button.userfg ? button.fgcol : (button.bgcol.r + button.bgcol.g + button.bgcol.b < 1) ? CHyprColor(0xFFFFFFFF) : CHyprColor(0xFF000000);
      hover.a = 0.18 * a;
      g_pHyprOpenGL->renderRect(buttonBox, hover, {.round = rounding});
    }

    offset += scaledButtonsPad + scaledButtonSize;
  }
}

void COmarchyFloatingHyprBar::renderBarButtonsText(CBox *barBox, const float scale, const float a) {
  const auto HEIGHT = g_pOmarchyFloatingGlobalState->config.barHeight->value();
  const auto BARBUTTONPADDING = g_pOmarchyFloatingGlobalState->config.barButtonPadding->value();
  const auto BARPADDING = g_pOmarchyFloatingGlobalState->config.barPadding->value();
  const auto ALIGNBUTTONS = g_pOmarchyFloatingGlobalState->config.barButtonsAlignment->value();
  const auto ICONONHOVER = g_pOmarchyFloatingGlobalState->config.iconOnHover->value();

  const bool BUTTONSRIGHT = ALIGNBUTTONS != "left";
  const auto visibleCount = getVisibleButtonCount(BARBUTTONPADDING, BARPADDING, Vector2D{barBox->w, barBox->h}, scale);
  const auto COORDS = cursorRelativeToBar();

  int offset = BARPADDING * scale;
  float noScaleOffset = BARPADDING;

  for (size_t i = 0; i < visibleCount; ++i) {
    auto &button = g_pOmarchyFloatingGlobalState->buttons[i];
    const auto scaledButtonSize = button.size * scale;
    const auto scaledButtonsPad = BARBUTTONPADDING * scale;

    // check if hovering here
    const auto BARBUF = Vector2D{(int)assignedBoxGlobal().w, HEIGHT};
    Vector2D currentPos =
        Vector2D{(BUTTONSRIGHT ? BARBUF.x - BARBUTTONPADDING - button.size - noScaleOffset : noScaleOffset), (BARBUF.y - button.size) / 2.0}.floor();
    bool hovering = VECINRECT(COORDS, currentPos.x, currentPos.y, currentPos.x + button.size + BARBUTTONPADDING, currentPos.y + button.size);
    noScaleOffset += BARBUTTONPADDING + button.size;

    if ((!button.iconTex || button.iconTex->m_texID == 0) && !button.icon.empty()) {
      // render icon
      auto fgcol = button.userfg ? button.fgcol : (button.bgcol.r + button.bgcol.g + button.bgcol.b < 1) ? CHyprColor(0xFFFFFFFF) : CHyprColor(0xFF000000);

      button.iconTex = g_pHyprRenderer->renderText(button.icon, fgcol, std::round(button.size * 0.62 * scale), false, "sans", scaledButtonSize);
    }

    if (!button.iconTex || button.iconTex->m_texID == 0)
      continue;

    const auto iconX =
        barBox->x + (BUTTONSRIGHT ? barBox->width - offset - scaledButtonSize / 2.0 : offset + scaledButtonSize / 2.0) - button.iconTex->m_size.x / 2.0;
    const auto iconY = barBox->y + barBox->height / 2.0 - button.iconTex->m_size.y / 2.0;
    CBox pos = {iconX, iconY, button.iconTex->m_size.x, button.iconTex->m_size.y};

    if (!ICONONHOVER || (ICONONHOVER && m_iButtonHoverState > 0))
      g_pHyprOpenGL->renderTexture(button.iconTex, pos, {.a = a});
    offset += scaledButtonsPad + scaledButtonSize;

    bool currentBit = (m_iButtonHoverState & (1 << i)) != 0;
    if (hovering != currentBit) {
      m_iButtonHoverState ^= (1 << i);
      // damage to get rid of some artifacts when icons are "hidden"
      damageEntire();
    }
  }
}

void COmarchyFloatingHyprBar::draw(PHLMONITOR pMonitor, const float &a) {
  const auto ENABLED = g_pOmarchyFloatingGlobalState->config.enabled->value();

  if (m_bLastEnabledState != ENABLED) {
    m_bLastEnabledState = ENABLED;
    g_pDecorationPositioner->repositionDeco(this);
  }

  // A client can switch between its own titlebar and ours at runtime, as
  // Chromium does when "Use system title bar and borders" is toggled.
  if (validMapped(m_pWindow) && clientDrawsDecorations(m_pWindow.lock()) != m_bClientDecorated) {
    updateRules();
  }

  if (m_hidden || !validMapped(m_pWindow) || !ENABLED)
    return;

  const auto PWINDOW = m_pWindow.lock();

  if (!PWINDOW->m_ruleApplicator->decorate().valueOrDefault())
    return;

  auto data = COmarchyFloatingBarPassElement::SBarData{this, a};
  g_pHyprRenderer->m_renderPass.add(makeUnique<COmarchyFloatingBarPassElement>(data));
}

void COmarchyFloatingHyprBar::renderPass(PHLMONITOR pMonitor, const float &a) {
  const auto PWINDOW = m_pWindow.lock();

  static auto PENABLEBLURGLOBAL = CConfigValue<Config::BOOL>("decoration:blur:enabled");
  const auto BARCOLOR = g_pOmarchyFloatingGlobalState->config.barColor->value();
  const auto HEIGHT = g_pOmarchyFloatingGlobalState->config.barHeight->value();
  const auto PRECEDENCE = g_pOmarchyFloatingGlobalState->config.barPrecedenceOverBorder->value();
  const auto ALIGNBUTTONS = g_pOmarchyFloatingGlobalState->config.barButtonsAlignment->value();
  const auto ENABLETITLE = g_pOmarchyFloatingGlobalState->config.barTitleEnabled->value();
  const auto ENABLEBLUR = g_pOmarchyFloatingGlobalState->config.barBlur->value();
  const auto INACTIVECOLOR = g_pOmarchyFloatingGlobalState->config.inactiveButtonColor->value();

  if (INACTIVECOLOR > 0) {
    bool currentWindowFocus = PWINDOW == Desktop::focusState()->window();
    if (currentWindowFocus != m_bWindowHasFocus) {
      m_bWindowHasFocus = currentWindowFocus;
      m_bButtonsDirty = true;
    }
  }

  const auto INACTIVEBAR = g_pOmarchyFloatingGlobalState->config.inactiveBarColor->value();
  const auto baseColor = PWINDOW != Desktop::focusState()->window() && INACTIVEBAR != 0 ? INACTIVEBAR : BARCOLOR;
  const CHyprColor DEST_COLOR = m_bForcedBarColor.value_or(configColor(baseColor));
  if (DEST_COLOR != m_cRealBarColor->goal())
    *m_cRealBarColor = DEST_COLOR;

  CHyprColor color = m_cRealBarColor->value();

  // A translucent window gets a bar of frosted glass to match its body.
  const bool GLASSY = glassy();
  if (GLASSY)
    color.a *= std::clamp(g_pOmarchyFloatingGlobalState->config.glassOpacity->value(), 0.F, 1.F);

  color.a *= a;
  const bool BUTTONSRIGHT = ALIGNBUTTONS != "left";
  const bool SHOULDBLUR = (ENABLEBLUR || GLASSY) && *PENABLEBLURGLOBAL && color.a < 1.F;

  if (HEIGHT < 1) {
    m_iLastHeight = HEIGHT;
    return;
  }

  const auto PWORKSPACE = PWINDOW->m_workspace;
  const auto WORKSPACEOFFSET = PWORKSPACE && !PWINDOW->m_pinned ? PWORKSPACE->m_renderOffset->value() : Vector2D();

  const auto ROUNDING = PWINDOW->rounding() + (PRECEDENCE ? 0 : PWINDOW->getRealBorderSize());

  const auto scaledRounding = ROUNDING > 0 ? ROUNDING * pMonitor->m_scale - 2 /* idk why but otherwise it looks bad due to the gaps */ : 0;

  m_seExtents = {{0, HEIGHT}, {}};

  const auto DECOBOX = assignedBoxGlobal();

  const auto BARBUF = DECOBOX.size() * pMonitor->m_scale;

  CBox titleBarBox = {DECOBOX.x - pMonitor->m_position.x, DECOBOX.y - pMonitor->m_position.y, DECOBOX.w,
                      DECOBOX.h + ROUNDING * 3 /* to fill the bottom cuz we can't disable rounding there */};

  titleBarBox.translate(PWINDOW->m_floatingOffset).scale(pMonitor->m_scale).round();

  if (titleBarBox.w < 1 || titleBarBox.h < 1)
    return;

  g_pHyprOpenGL->scissor(titleBarBox);

  if (ROUNDING) {
    // the +1 is a shit garbage temp fix until renderRect supports an alpha matte
    CBox windowBox = {PWINDOW->position(Desktop::View::IGeometric::GEOMETRIC_CURRENT).x + PWINDOW->m_floatingOffset.x - pMonitor->m_position.x + 1,
                      PWINDOW->position(Desktop::View::IGeometric::GEOMETRIC_CURRENT).y + PWINDOW->m_floatingOffset.y - pMonitor->m_position.y + 1,
                      PWINDOW->size(Desktop::View::IGeometric::GEOMETRIC_CURRENT).x - 2, PWINDOW->size(Desktop::View::IGeometric::GEOMETRIC_CURRENT).y - 2};

    if (windowBox.w < 1 || windowBox.h < 1)
      return;

    // Hyprland's own stencil passes (blurred surfaces that discard by alpha,
    // such as the glassy layers) leave the stencil write mask at 0, and a masked
    // clear clears nothing. The bar would then be cut by whatever stencil they
    // left behind and show the window's shadow through instead of its color.
    glStencilMask(0xFF);
    glClearStencil(0);
    glClear(GL_STENCIL_BUFFER_BIT);

    g_pHyprOpenGL->setCapStatus(GL_STENCIL_TEST, true);

    glStencilFunc(GL_ALWAYS, 1, -1);
    glStencilOp(GL_KEEP, GL_KEEP, GL_REPLACE);

    glColorMask(GL_FALSE, GL_FALSE, GL_FALSE, GL_FALSE);

    windowBox.translate(WORKSPACEOFFSET).scale(pMonitor->m_scale).round();
    g_pHyprOpenGL->renderRect(windowBox, CHyprColor(0, 0, 0, 0), {.round = scaledRounding, .roundingPower = m_pWindow->roundingPower()});
    glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);

    glStencilFunc(GL_NOTEQUAL, 1, -1);
    glStencilOp(GL_KEEP, GL_KEEP, GL_REPLACE);
  }

  g_pHyprOpenGL->renderRect(titleBarBox, color, {.round = static_cast<int>(scaledRounding), .roundingPower = PWINDOW->roundingPower(), .blur = SHOULDBLUR, .blurA = a});

  // The app icon sits before the title, sized to the text and kept inside the bar.
  const auto TEXTSIZE = g_pOmarchyFloatingGlobalState->config.barTextSize->value();
  const int iconPx = g_pOmarchyFloatingGlobalState->config.barIcon->value()
                         ? std::clamp(static_cast<int>(std::round(TEXTSIZE * 1.5 * pMonitor->m_scale)), 0, std::max(0, static_cast<int>(BARBUF.y - 4 * pMonitor->m_scale)))
                         : 0;
  updateIcon(PWINDOW, iconPx);
  const bool HASICON = iconPx > 0 && m_pIconTex && m_pIconTex->m_texID != 0;
  const int iconGap = std::round(6 * pMonitor->m_scale);
  const int iconWidth = HASICON ? iconPx + iconGap : 0;

  // render title
  const std::string title = currentTitle(PWINDOW);
  const std::string prefix = currentPrefix(PWINDOW, title);
  // The text is fitted to the room the icon leaves, so it is rebuilt when that changes too.
  if (ENABLETITLE && (m_szLastTitle != title || m_szLastPrefix != prefix || m_iLastIconWidth != iconWidth || m_bWindowSizeChanged || !m_pTextTex ||
                      m_pTextTex->m_texID == 0 || m_bTitleColorChanged)) {
    m_szLastTitle = title;
    m_szLastPrefix = prefix;
    m_iLastIconWidth = iconWidth;
    renderBarTitle(BARBUF, pMonitor->m_scale, iconWidth);
  }

  if (ROUNDING) {
    // cleanup stencil
    glClearStencil(0);
    glClear(GL_STENCIL_BUFFER_BIT);
    g_pHyprOpenGL->setCapStatus(GL_STENCIL_TEST, false);
    glStencilMask(-1);
    glStencilFunc(GL_ALWAYS, 1, 0xFF);
  }

  CBox textBox = {titleBarBox.x, titleBarBox.y, (int)BARBUF.x, (int)BARBUF.y};
  if (HASICON || (ENABLETITLE && m_pTextTex)) {
    const auto BARPADDING = g_pOmarchyFloatingGlobalState->config.barPadding->value();
    const auto BARBUTTONPADDING = g_pOmarchyFloatingGlobalState->config.barButtonPadding->value();
    const auto ALIGN = g_pOmarchyFloatingGlobalState->config.barTextAlign->value();

    float buttonSizes = BARBUTTONPADDING;
    for (auto &b : g_pOmarchyFloatingGlobalState->buttons) {
      buttonSizes += b.size + BARBUTTONPADDING;
    }

    const auto scaledBorderSize = PWINDOW->getRealBorderSize() * pMonitor->m_scale;
    const auto scaledButtonsSize = buttonSizes * pMonitor->m_scale;
    const auto scaledBarPadding = BARPADDING * pMonitor->m_scale;
    const bool hasText = ENABLETITLE && m_pTextTex;
    const bool hasPrefix = hasText && m_pPrefixTex;
    const double prefixWidth = hasPrefix ? m_pPrefixTex->m_size.x + std::round(g_pOmarchyFloatingGlobalState->config.barTextSize->value() * 0.6 * pMonitor->m_scale) : 0;
    const double textWidth = hasText ? prefixWidth + m_pTextTex->m_size.x : 0;
    const double contentWidth = iconWidth + textWidth - (HASICON && !hasText ? iconGap : 0);
    const auto xOffset = ALIGN == "left" ? std::round(scaledBarPadding + (BUTTONSRIGHT ? 0 : scaledButtonsSize))
                                         : std::round(((BARBUF.x - scaledBorderSize) / 2.0 - contentWidth / 2.0));

    if (HASICON) {
      // Fit the icon in its square without stretching it: not every icon is square.
      const auto texSize = m_pIconTex->m_size;
      const double fit = texSize.x > 0 && texSize.y > 0 ? std::min(iconPx / texSize.x, iconPx / texSize.y) : 1.0;
      const double iconW = std::round(texSize.x * fit), iconH = std::round(texSize.y * fit);
      CBox iconBox = {textBox.x + xOffset + std::round((iconPx - iconW) / 2.0), textBox.y + std::round((BARBUF.y - iconH) / 2.0), iconW, iconH};
      g_pHyprOpenGL->renderTexture(m_pIconTex, iconBox, {.a = a});
    }

    // The app's name and separator are dimmed, so the title stands out.
    if (hasPrefix) {
      const auto yOffset = std::round((BARBUF.y - m_pPrefixTex->m_size.y) / 2.0);
      CBox prefixBox = {textBox.x + xOffset + iconWidth, textBox.y + yOffset, m_pPrefixTex->m_size.x, m_pPrefixTex->m_size.y};
      g_pHyprOpenGL->renderTexture(m_pPrefixTex, prefixBox, {.a = a * 0.5f});
    }

    if (hasText) {
      const auto yOffset = std::round((BARBUF.y - m_pTextTex->m_size.y) / 2.0);
      CBox titleBox = {textBox.x + xOffset + iconWidth + prefixWidth, textBox.y + yOffset, m_pTextTex->m_size.x, m_pTextTex->m_size.y};
      g_pHyprOpenGL->renderTexture(m_pTextTex, titleBox, {.a = a});
    }
  }

  renderBarButtons(&textBox, pMonitor->m_scale, a);
  m_bButtonsDirty = false;

  g_pHyprOpenGL->scissor(nullptr);

  renderBarButtonsText(&textBox, pMonitor->m_scale, a);

  m_bWindowSizeChanged = false;
  m_bTitleColorChanged = false;

  // dynamic updates change the extents
  if (m_iLastHeight != HEIGHT) {
    PWINDOW->layoutTarget()->recalc();
    m_iLastHeight = HEIGHT;
  }
}

// The window's own title follows what the app is doing, such as the page or file
// it has open. "app" titles the bar with the app's name instead, which is
// steadier for web apps whose title is just their domain.
std::string COmarchyFloatingHyprBar::currentTitle(PHLWINDOW window) {
  const bool byApp = g_pOmarchyFloatingGlobalState->config.barTitleSource->value() == "app";
  std::string title = byApp ? omarchyAppName(window->m_class) : window->m_title;
  if (title.empty())
    title = byApp ? window->m_title : omarchyAppName(window->m_class);
  return title;
}

// The app's name before the window's title, as in "Foot ›", when the title
// doesn't name the app already: a browser's title usually ends with it, and
// many apps title their only window with just their name.
std::string COmarchyFloatingHyprBar::currentPrefix(PHLWINDOW window, const std::string &title) {
  const auto separator = g_pOmarchyFloatingGlobalState->config.barTitleSeparator->value();
  if (separator.empty() || title.empty() || g_pOmarchyFloatingGlobalState->config.barTitleSource->value() == "app")
    return "";

  const auto app = omarchyAppName(window->m_class);
  if (app.empty())
    return "";

  auto lower = [](std::string text) {
    std::ranges::transform(text, text.begin(), [](unsigned char c) { return std::tolower(c); });
    return text;
  };
  // A path remains explanatory even if a directory is named after the app.
  if (!title.starts_with('/') && lower(title).contains(lower(app)))
    return "";

  return app + " " + separator;
}

void COmarchyFloatingHyprBar::updateIcon(PHLWINDOW window, int pixels) {
  if (pixels <= 0) {
    m_pIconTex = nullptr;
    m_szIconPath.clear();
    return;
  }

  // The lookup is cached and rescans the desktop entries every couple of
  // seconds, so an entry installed after the window opened still reaches it.
  const auto path = omarchyAppIcon(window->m_class);
  if (path == m_szIconPath && m_iIconPx == pixels)
    return;

  m_szIconPath = path;
  m_iIconPx = pixels;
  m_pIconTex = nullptr;

  if (path.empty())
    return;

  Hyprgraphics::CImage image(path, Vector2D{(double)pixels, (double)pixels});
  if (!image.success() || !image.cairoSurface() || !image.cairoSurface()->cairo()) {
    Log::logger->log(Log::DEBUG, "[hyprbars] Could not load icon {}: {}", path, image.getError());
    return;
  }

  m_pIconTex = g_pHyprRenderer->createTexture(image.cairoSurface()->cairo());
}

eDecorationType COmarchyFloatingHyprBar::getDecorationType() { return DECORATION_CUSTOM; }

void COmarchyFloatingHyprBar::updateWindow(PHLWINDOW pWindow) {
  updateRules();
  damageEntire();
}

void COmarchyFloatingHyprBar::onConfigReloaded() {
  m_bButtonsDirty = true;
  m_bTitleColorChanged = true;
  m_pTextTex = nullptr;
  m_pPrefixTex = nullptr;
  m_pIconTex = nullptr;
  m_szIconPath.clear();

  g_pDecorationPositioner->repositionDeco(this);

  // Bars start disabled, so the reload that enables them has to make room for
  // them, as a change in m_hidden does.
  const auto ENABLED = g_pOmarchyFloatingGlobalState->config.enabled->value();
  if (m_bLastEnabledState != ENABLED) {
    m_bLastEnabledState = ENABLED;
    if (const auto PWINDOW = m_pWindow.lock(); PWINDOW)
      if (const auto monitor = PWINDOW->m_monitor.lock())
        monitor->m_scheduledRecalc = true;
  }

  damageEntire();
}

void COmarchyFloatingHyprBar::damageEntire() { g_pHyprRenderer->damageBox(assignedBoxGlobal()); }

Vector2D COmarchyFloatingHyprBar::cursorRelativeToBar() { return g_pInputManager->getMouseCoordsInternal() - assignedBoxGlobal().pos(); }

eDecorationLayer COmarchyFloatingHyprBar::getDecorationLayer() { return DECORATION_LAYER_UNDER; }

uint64_t COmarchyFloatingHyprBar::getDecorationFlags() {
  return DECORATION_ALLOWS_MOUSE_INPUT | (g_pOmarchyFloatingGlobalState->config.barPartOfWindow->value() ? DECORATION_PART_OF_MAIN_WINDOW : 0);
}

CBox COmarchyFloatingHyprBar::assignedBoxGlobal() {
  if (!validMapped(m_pWindow))
    return {};

  CBox box = m_bAssignedBox;
  box.translate(g_pDecorationPositioner->getEdgeDefinedPoint(DECORATION_EDGE_TOP, m_pWindow.lock()));

  const auto PWORKSPACE = m_pWindow->m_workspace;
  const auto WORKSPACEOFFSET = PWORKSPACE && !m_pWindow->m_pinned ? PWORKSPACE->m_renderOffset->value() : Vector2D();

  return box.translate(WORKSPACEOFFSET);
}

PHLWINDOW COmarchyFloatingHyprBar::getOwner() { return m_pWindow.lock(); }

// Whether the window is one Omarchy makes see-through (glass_tag), whose bar is
// drawn as frosted glass instead of a solid color.
bool COmarchyFloatingHyprBar::glassy() {
  const auto PWINDOW = m_pWindow.lock();
  const auto tag = g_pOmarchyFloatingGlobalState->config.glassTag->value();
  return PWINDOW && !tag.empty() && PWINDOW->m_ruleApplicator->m_tagKeeper.isTagged(tag);
}

void COmarchyFloatingHyprBar::updateRules() {
  const auto PWINDOW = m_pWindow.lock();
  if (!PWINDOW)
    return;
  auto prevHidden = m_hidden;
  auto prevForcedTitleColor = m_bForcedTitleColor;

  m_bForcedBarColor = std::nullopt;
  m_bForcedTitleColor = std::nullopt;
  m_hidden = false;

  if (PWINDOW->m_ruleApplicator->m_otherProps.props.contains(g_pOmarchyFloatingGlobalState->nobarRuleIdx))
    m_hidden = truthy(PWINDOW->m_ruleApplicator->m_otherProps.props.at(g_pOmarchyFloatingGlobalState->nobarRuleIdx)->effect);
  m_bClientDecorated = clientDrawsDecorations(PWINDOW);
  if (m_bClientDecorated && g_pOmarchyFloatingGlobalState->config.respectClientDecorations->value())
    m_hidden = true;
  const auto workspaceTag = g_pOmarchyFloatingGlobalState->config.workspaceTag->value();
  if (!workspaceTag.empty())
    m_hidden = m_hidden || !PWINDOW->m_isFloating || !PWINDOW->m_ruleApplicator->m_tagKeeper.isTagged(workspaceTag);
  if (PWINDOW->m_ruleApplicator->m_otherProps.props.contains(g_pOmarchyFloatingGlobalState->barColorRuleIdx))
    m_bForcedBarColor = CHyprColor(
        Config::ParserUtils::parseColor(PWINDOW->m_ruleApplicator->m_otherProps.props.at(g_pOmarchyFloatingGlobalState->barColorRuleIdx)->effect).value_or(0));
  if (PWINDOW->m_ruleApplicator->m_otherProps.props.contains(g_pOmarchyFloatingGlobalState->titleColorRuleIdx))
    m_bForcedTitleColor =
        CHyprColor(Config::ParserUtils::parseColor(PWINDOW->m_ruleApplicator->m_otherProps.props.at(g_pOmarchyFloatingGlobalState->titleColorRuleIdx)->effect)
                       .value_or(0));

  const bool MINIMIZE = onBarWorkspace(PWINDOW) && !g_pOmarchyFloatingGlobalState->config.onMinimizeRequest->value().empty();
  if (m_bAdvertisedMinimize != MINIMIZE) {
    m_bAdvertisedMinimize = MINIMIZE;
    advertiseMinimize(PWINDOW, MINIMIZE);
  }

  if (prevHidden != m_hidden) {
    g_pDecorationPositioner->repositionDeco(this);
    if (const auto monitor = PWINDOW->m_monitor.lock())
      monitor->m_scheduledRecalc = true;
  }
  if (prevForcedTitleColor != m_bForcedTitleColor)
    m_bTitleColorChanged = true;
}

void COmarchyFloatingHyprBar::damageOnButtonHover() {
  const auto BARPADDING = g_pOmarchyFloatingGlobalState->config.barPadding->value();
  const auto BARBUTTONPADDING = g_pOmarchyFloatingGlobalState->config.barButtonPadding->value();
  const auto HEIGHT = g_pOmarchyFloatingGlobalState->config.barHeight->value();
  const auto ALIGNBUTTONS = g_pOmarchyFloatingGlobalState->config.barButtonsAlignment->value();
  const bool BUTTONSRIGHT = ALIGNBUTTONS != "left";

  float offset = BARPADDING;

  const auto COORDS = cursorRelativeToBar();

  unsigned int hoverState = 0;
  for (size_t i = 0; i < g_pOmarchyFloatingGlobalState->buttons.size(); ++i) {
    const auto &b = g_pOmarchyFloatingGlobalState->buttons[i];
    const auto BARBUF = Vector2D{(int)assignedBoxGlobal().w, HEIGHT};
    Vector2D currentPos = Vector2D{(BUTTONSRIGHT ? BARBUF.x - BARBUTTONPADDING - b.size - offset : offset), (BARBUF.y - b.size) / 2.0}.floor();

    if (VECINRECT(COORDS, currentPos.x, currentPos.y, currentPos.x + b.size + BARBUTTONPADDING, currentPos.y + b.size))
      hoverState |= 1u << i;

    offset += BARBUTTONPADDING + b.size;
  }

  if (hoverState != m_iButtonHoverState) {
    m_iButtonHoverState = hoverState;
    damageEntire();
  }
}
