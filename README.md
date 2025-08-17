# of-vscode-project-generator-rs

[![Latest Version](https://img.shields.io/github/tag/funatsufumiya/of-vscode-project-generator-rs.svg?style=flat-square)](https://github.com/funatsufumiya/of-vscode-project-generator-rs/tags)

***WIP, Experimental***

openFrameworks project generator for Visual Studio Code. (Rust ported version of [original bash version](https://github.com/funatsufumiya/of-vscode-project-generator))

(only for syntax-highlighting and intellisense. Not for building or debug.)

## Usage

NOTE: You first need to generate project using default projectGenerator.

```bash
$ cd /path/to/your/openFrameworks/apps/myApps
$ cd your_project
$ of-vscode-project-generator-rs .
```

## Install

```bash
$ git clone https://github.com/funatsufumiya/of-vscode-project-generator
$ cd of-vscode-project-generator
$ cargo install --path .
```

## Limitations

- This script loads some of `addon_config.mk` incompletedly (and not load `config.make`). If you need more, please modify `.vscode/c_cpp_properties.json` manually after running this script.
- This script exports environment-dependent settings. So I recommend NOT to include `.vscode` directory in your git repository.

## Note

Code port from bash into Rust is mainly done by GitHub Copilot.<br>
Already tested well, but use with care.

## License

WTFPL or 0BSD