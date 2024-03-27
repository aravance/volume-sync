# volume-sync

A rust utility to keep sink volumes in sync written using [libpulse_binding](https://docs.rs/libpulse-binding/latest/libpulse_binding/)

It uses a config file with the following priority:
1. `$XDG_CONFIG_HOME/volume-sync.toml`
2. `$HOME/.config/volume-sync.toml`

Config options:
```
log_level: Off|Error|Warn|Info|Debug|Trace - default:Info -- log level
sinks: array<string> -- list of sink names to keep in sync
```

e.g.
```toml
log_level = "Info"
sinks = [
  "alsa_output.usb-Audeze_LLC_Audeze_Maxwell_Dongle_0000000000000000-01.pro-output-0",
  "alsa_output.usb-Audeze_LLC_Audeze_Maxwell_Dongle_0000000000000000-01.pro-output-1",
]
```

## Get sink names
If for example you have Audeze Maxwell with a chat and game channel that you want to keep in sync.

First use pactl to list sinks and find the `Name`
```bash
$ pactl list sinks
...
Sink #108
        State: SUSPENDED
        Name: alsa_output.usb-Audeze_LLC_Audeze_Maxwell_Dongle_0000000000000000-01.pro-output-0
        Description: Audeze Maxwell Chat
        Driver: PipeWire
...

Sink #109
        State: SUSPENDED
        Name: alsa_output.usb-Audeze_LLC_Audeze_Maxwell_Dongle_0000000000000000-01.pro-output-1
        Description: Audeze Maxwell Game
        Driver: PipeWire
...
```

Add the sink `Name` to an array in the toml config file
```bash
cat >~/.config/volume-sync.toml <<EOF
sinks: [
  "alsa_output.usb-Audeze_LLC_Audeze_Maxwell_Dongle_0000000000000000-01.pro-output-0",
  "alsa_output.usb-Audeze_LLC_Audeze_Maxwell_Dongle_0000000000000000-01.pro-output-1",
]
EOF
```
