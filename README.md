# of-vscode-project-generator

[![Latest Version](https://img.shields.io/github/tag/funatsufumiya/of-vscode-project-generator.svg?style=flat-square)](https://github.com/funatsufumiya/of-vscode-project-generator/tags)

openFrameworks project generator for Visual Studio Code.

(only for syntax-highlighting and intellisense. Not for building or debug.)

## Usage

NOTE: You first need to generate project using default projectGenerator.

```bash
$ cd /path/to/your/openFrameworks/apps/myApps
$ cd your_project
$ of-vscode-project-generator
```

For windows users, please use git-bash.

## Install

```bash
$ git clone https://github.com/funatsufumiya/of-vscode-project-generator
$ cd of-vscode-project-generator
$ chmod +x of-vscode-project-generator.sh
$ cp of-vscode-project-generator.sh /usr/local/bin/of-vscode-project-generator

# NOTE: For windows user, on git-bash, /usr/local/bin/ may not exist.
#       Please copy to /usr/bin/ or any directory in PATH.
```

## Limitations

- This script loads some of `addon_config.mk` (and not load `config.make`). If you need more, please modify `.vscode/c_cpp_properties.json` manually after running this script.
- This script exports environment-dependent settings. So I recommend NOT to include `.vscode` directory in your git repository.
