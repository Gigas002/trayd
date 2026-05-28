#!/usr/bin/env nu

# Stream tray events from trayd as structured nushell records.
# Press Ctrl+C to stop.
#
# Usage:
#   nu watch-events.nu
#   nu watch-events.nu | where type == "item_added"
#   nu watch-events.nu | where { |e| $e.item?.status? == "needs_attention" }

def main [] {
    ^trayd subscribe
    | lines
    | each { |line| $line | from json }
}
