#!/usr/bin/env nu

# Navigate a tray item's DBusMenu using rofi (or any dmenu-compatible launcher).
# Handles submenus: the launcher is re-invoked for each level.
#
# Usage:
#   nu menu-rofi.nu ':1.118/StatusNotifierItem'
#   nu menu-rofi.nu ':1.118/StatusNotifierItem' --launcher 'wofi --dmenu'
#
# Get item ids with: trayd list

def main [
    item: string
    --launcher: string = "rofi -dmenu"
] {
    mut node_arg: list<string> = []

    loop {
        let label = (
            ^trayd menu-list --item $item ...$node_arg
            | ^sh -c $launcher
            | str trim
        )

        if ($label | is-empty) { break }

        let result = (
            ^trayd menu-click --item $item --label $label ...$node_arg
            | complete
        )

        match $result.exit_code {
            0 => { break }
            2 => { $node_arg = ["--node" ($result.stdout | str trim)] }
            _ => { break }
        }
    }
}
