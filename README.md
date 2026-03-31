# pino

A desktop app for exporting audio libraries to Pioneer-compatible USB drives, powered by the [rekordcrate](https://github.com/Holzhaus/rekordcrate) library.

Built with [Dioxus](https://dioxuslabs.com/).

## Features

- SQLite-backed music library
- Audio format conversion via FFmpeg (MP3, M4A, etc.)
- Bidirectional sync with USB devices (pull from device, push to device)
- Metadata extraction (currently just title, artist and album)

## Installation

### Pre-requisites

FFmpeg is required for audio format conversion. Please find out how to install it on your platform [here](https://ffmpeg.org/download.html).

### Option A: Pre-built binaries

You can download pre-built binaries of the app from the [releases page](https://github.com/acrilique/pino/releases).

### Option B: Building from source

## Requirements

- Rust. To install it on your system, follow the instructions on the [official Rust website](https://www.rust-lang.org/tools/install).
- Dioxus CLI. For this one, check the documentation on the [Dioxus website](https://dioxuslabs.com/learn/0.7/getting_started). Make sure to check the "Platform-specific dependencies" section for your OS.

## Getting Started

Serve the app:

```sh
dx serve --release
```

Bundle the app:

```sh
dx bundle --release
```