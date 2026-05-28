#!/usr/bin/env nu

# Navigate a tray item's DBusMenu using nushell's built-in input list.
# No external dependencies required.
#
# Usage:
#   nu menu-interactive.nu ':1.118/StatusNotifierItem'
#
# Get item ids with: trayd list

def main [item: string] {
    mut node_arg: list<string> = []

    loop {
        let labels = (
            ^trayd menu-list --item $item ...$node_arg
            | lines
            | where { |l| ($l | str length) > 0 }
        )

        if ($labels | is-empty) { break }

        let label = ($labels | input list --fuzzy "Select")
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
