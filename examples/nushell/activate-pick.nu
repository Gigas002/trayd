#!/usr/bin/env nu

# Pick a tray item interactively and activate it (primary left-click).
#
# Usage:
#   nu activate-pick.nu

def main [] {
    let items = (^trayd list | lines | where { |l| ($l | str length) > 0 })

    if ($items | is-empty) {
        print "No tray items registered."
        return
    }

    let choice = ($items | input list --fuzzy "Activate item")
    if ($choice | is-empty) { return }

    # Line format: '<id>: <title> [<status>]' — take everything before the first ': '
    let id = ($choice | split row ": " | first)
    ^trayd activate $id
}
