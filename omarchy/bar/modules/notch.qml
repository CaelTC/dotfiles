import QtQuick
import QtQuick.Window

// Center anchor that reserves the MacBook camera notch on the built-in panel
// and collapses to nothing on external displays, so the center widgets close
// up around the middle instead of straddling an empty gap.
// ponytail: eDP prefix, not the aspect-ratio test BarModel.notchHeight uses.
Item {
  property var bar
  property string moduleName
  property var settings

  readonly property int span: String(Screen.name).indexOf("eDP") === 0
    ? (settings && settings.size !== undefined ? Number(settings.size) : 230)
    : 0

  implicitWidth: span
  implicitHeight: bar ? bar.barSize : 26
}
