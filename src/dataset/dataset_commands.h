#pragma once

// Dataset subcommand handlers for ego-recorder CLI.
//
// Called from main.cpp subcommand dispatch. Each returns an exit code (0 = success).

/// `ego-recorder dataset init -o DIR --name NAME [--description DESC] [--tags t1,t2] [--force]`
int cmd_dataset_init(int argc, char* argv[]);

/// `ego-recorder dataset info DIR`
int cmd_dataset_info(int argc, char* argv[]);

/// `ego-recorder dataset add DIR file.egorec [...]`
int cmd_dataset_add(int argc, char* argv[]);

/// `ego-recorder dataset remove DIR filename.egorec`
int cmd_dataset_remove(int argc, char* argv[]);
