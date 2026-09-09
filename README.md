# Cosmic Ext Thunderbolt

A Thunderbolt™ applet for the COSMIC™ desktop

    TODO: insert screenshot

## Origins

Project was initialized using the [cosmic-applet-template](https://github.com/pop-os/cosmic-applet-template) and
the initial UI/logic iteration was heavily inspired by the official
[cosmic-applet-bluetooth](https://github.com/pop-os/cosmic-applets/tree/master/cosmic-applet-bluetooth).

Consequently, some source files retain copyright notices from System76 in accordance with the GPL-3.0-only license.

## Requirements

- [COSMIC™ Desktop Environment](https://system76.com/cosmic)
- [bolt thunderbolt device manager](https://gitlab.freedesktop.org/bolt/bolt) installed
  - `boltd` running
- [Rust toolchain](https://rust-lang.org/tools/install/) for building from source

## Installation

A [justfile](./justfile) is included by default for the [casey/just][just] command runner.

- `just` builds the application with the default `just build-release` recipe
- `just run` builds and runs the application
- `just install` installs the project into the system
- `just vendor` creates a vendored tarball
- `just build-vendored` compiles with vendored dependencies from that tarball
- `just check` runs clippy on the project to check for linter warnings
- `just check-json` can be used by IDEs that support LSP

## Configuration

Advanced users may wish to display the Thunderbolt Host Controller in the applet list. This is hidden by default.

To show it:
1. create or edit the configuration file  
`~/.config/cosmic/fr.doursenaud.raphael.cosmic-ext-thunderbolt/v1/show_host_device`
2. set it to `true`

## Translators

[Fluent][fluent] is used for localization of the software. Fluent's translation files are found in the [i18n directory](./i18n). New translations may copy the [English (en) localization](./i18n/en) of the project, rename `en` to the desired [ISO 639-1 language code][iso-codes], and then translations can be provided for each [message identifier][fluent-guide]. If no translation is necessary, the message may be omitted.

## Packaging

If packaging for a Linux distribution, vendor dependencies locally with the `vendor` rule, and build with the vendored sources using the `build-vendored` rule. When installing files, use the `rootdir` and `prefix` variables to change installation paths.

```sh
just vendor
just build-vendored
just rootdir=debian/cosmic-ext-thunderbolt prefix=/usr install
```

It is recommended to build a source tarball with the vendored dependencies, which can typically be done by running `just vendor` on the host system before it enters the build environment.

## Developers

Developers should install [rustup][rustup] and configure their editor to use [rust-analyzer][rust-analyzer].

[fluent]: https://projectfluent.org/
[fluent-guide]: https://projectfluent.org/fluent/guide/hello.html
[iso-codes]: https://en.wikipedia.org/wiki/List_of_ISO_639-1_codes
[just]: https://github.com/casey/just
[rustup]: https://rustup.rs/
[rust-analyzer]: https://rust-analyzer.github.io/
[sccache]: https://github.com/mozilla/sccache
