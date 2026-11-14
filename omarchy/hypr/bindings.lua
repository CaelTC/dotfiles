-- Keep only your personal keybinding overrides here. Add new bindings with
-- o.bind or replace defaults with o.rebind.

-- See current bindings and descriptions:
--   omarchy menu keybindings --print

-- To disable every Omarchy default binding, set this in
-- ~/.config/hypr/hyprland.lua before require("default.hypr.omarchy"), then add
-- only the bindings you want below:
--   omarchy_default_bindings = false

-- To disable all preinstalled app/webapp bindings, set:
--   omarchy_preinstalled_bindings = false

-- Add a new binding.
-- o.bind("SUPER + SHIFT + R", "SSH", "alacritty -e ssh your-server")

-- Change an existing binding. o.rebind takes the same arguments as o.bind.
-- This example replaces the default file manager with Flea.
-- o.rebind("SUPER + SHIFT + F", "File manager", { launch = "flea" })

-- Disable a default binding without replacing it.
-- hl.unbind("SUPER + SHIFT + B")

-- Logitech MX Keys examples:
-- o.bind("SUPER + H", nil, "voxtype record toggle")
-- o.bind("SUPER + PERIOD", nil, { panel = "omarchy.emojis" })

o.bind("SUPER + SHIFT + L", "La Presse", { webapp = "https://www.lapresse.ca/" })
o.rebind("SUPER + SHIFT + O", "Odoo", { webapp = "https://nootka.odoo.com" })
o.rebind("SUPER + SHIFT + A", "Avalanche Canada", { webapp = "https://www.avalanche.ca/" })
-- SUPER+CTRL+RETURN (was: Herdr TUI in a terminal) opens or raises the Herdr desktop app.
o.rebind("SUPER + CTRL + RETURN", "Herdr", { launch = "herdr-app" })

-- Reopen the last closed app ("P for previous"). See hypr/reopen.lua.
local reopen = require("hypr.reopen")
o.bind("SUPER + ALT + P", "Reopen last closed app", reopen.reopen)

-- Ctrl+Alt+Tab switches screens and flashes a monitor icon in the middle of the
-- one now focused (drawn by the cael.focusflash shell plugin).
local function focus_monitor_and_flash(direction)
  return function()
    hl.dispatch(hl.dsp.focus({ monitor = direction }))
    hl.exec_cmd("omarchy-shell -q focusflash show")
  end
end
o.rebind("CTRL + ALT + TAB", "Focus on next monitor", focus_monitor_and_flash("+1"))
o.rebind("CTRL + ALT + SHIFT + TAB", "Focus on previous monitor", focus_monitor_and_flash("-1"))
