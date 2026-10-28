#!/bin/sh
# Example data source for the Game Friends widget: prints JSON on stdout.
# Swap this for a real query (a game launcher's CLI, a local API) and keep the same shape.
# It runs as you, in this folder, for at most the widget's timeout; nothing else reads it.
cat <<'JSON'
{
  "total": 12,
  "online": [
    {"name": "Alice", "game": "Croquet Royale", "status": "In a match"},
    {"name": "Dinah", "game": "Yarn Quest", "status": "Exploring"},
    {"name": "Hatter", "game": "Tea Tycoon", "status": "Lobby"}
  ],
  "week": [2, 3, 1, 4, 6, 5, 3]
}
JSON
