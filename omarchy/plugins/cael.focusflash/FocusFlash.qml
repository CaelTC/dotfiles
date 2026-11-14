import QtQuick
import Quickshell
import Quickshell.Wayland
import qs.Commons
import qs.Commons as Commons
import qs.Ui

// Shows a monitor glyph centered on the focused screen for half a second.
// Hyprland calls `omarchy-shell -q focusflash show` on every monitor focus
// change (see hypr/looknfeel.lua).
Item {
  id: root

  property bool opened: false
  readonly property int pad: Style.space(18)

  function flash() {
    // Reopen so OverlayWindow re-targets the newly focused screen.
    if (opened) {
      opened = false
      Qt.callLater(function() { root.opened = true; hideTimer.restart() })
    } else {
      opened = true
      hideTimer.restart()
    }
  }

  Timer {
    id: hideTimer
    interval: 500
    onTriggered: root.opened = false
  }

  ShellIpc {
    target: "focusflash"
    function show(): string { root.flash(); return "ok" }
    function ping(): string { return "ok" }
  }

  OverlayWindow {
    id: panel
    shown: root.opened
    shownKeyboardFocus: WlrKeyboardFocus.None
    WlrLayershell.namespace: "cael-focusflash"
    // Visual only: never block clicks.
    mask: Region {}

    BorderSurface {
      id: card
      width: card.borderLeft + root.pad + glyph.implicitWidth + root.pad + card.borderRight
      height: card.borderTop + root.pad + glyph.implicitHeight + root.pad + card.borderBottom
      anchors.centerIn: parent
      color: Util.alpha(Commons.Color.background, 0.92)
      borderSpec: Border.surfaceSpec("popups", "border", Commons.Color.popups.border, Math.max(1, Style.space(2)))
      radius: Style.cornerRadius

      Text {
        id: glyph
        anchors.centerIn: parent
        textFormat: Text.PlainText
        text: "\u{F0379}" // nf-md-monitor
        color: Commons.Color.foreground
        font.family: Style.font.family
        font.pixelSize: Style.font.displayLarge
      }
    }
  }
}
