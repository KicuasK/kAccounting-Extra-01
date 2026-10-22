#!/bin/sh
# Compile only; no installation, panel, or existing-output replacement.
set -eu
script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec /usr/bin/python3 "$script_directory/build-native-picker.py" "$@"
