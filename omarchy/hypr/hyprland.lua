-- Learn how to configure Hyprland: https://wiki.hypr.land/Configuring/Start/

-- Omarchy's bootstrap keeps path setup out of this user config.
dofile((os.getenv("OMARCHY_PATH") or "/usr/share/omarchy") .. "/default/hypr/bootstrap.lua")

-- Disable all Omarchy default bindings. Add your own in hypr/bindings.lua.
-- omarchy_default_bindings = false
--
-- Or disable only bindings for Omarchy's preinstalled apps/web apps while
-- keeping core window-manager bindings:
-- omarchy_preinstalled_bindings = false

-- Load Omarchy defaults.
require("default.hypr.omarchy")

-- Put your personal overrides in these files. They're loaded after Omarchy's
-- defaults so package updates can improve the defaults without rewriting your
-- ~/.config/hypr files.
require("hypr.monitors")
require("hypr.input")
require("hypr.bindings")
require("hypr.looknfeel")
require("hypr.autostart")

-- Toggle config flags dynamically.
require("default.hypr.toggles")

-- Add any other personal Hyprland configuration below.
-- o.window("qemu", { workspace = "5" })

-- Keep the screensaver off while a Chromium window (incl. web apps) is fullscreen.
o.window("chrom.*", { idle_inhibit = "fullscreen" })

-- Local overrides of omarchy-* commands (clock screensaver). Must precede $OMARCHY_PATH/bin.
hl.env("PATH", os.getenv("HOME") .. "/.local/share/omarchy-overrides/bin:" .. (os.getenv("OMARCHY_PATH") or "/usr/share/omarchy") .. "/bin:" .. os.getenv("PATH"))
