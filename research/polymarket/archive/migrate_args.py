import sys

bak_path = r'c:\Users\mehul\OneDrive\Desktop\Studio\PROJECTS\polycli\polycli\src\commands\bot.rs.bak'
bot_path = r'c:\Users\mehul\OneDrive\Desktop\Studio\PROJECTS\polycli\polycli\src\commands\bot.rs'
discovery_path = r'c:\Users\mehul\OneDrive\Desktop\Studio\PROJECTS\polycli\polycli\src\bot\discovery\mod.rs'

with open(bak_path, 'r', encoding='utf-8') as f:
    lines = f.readlines()

# Extract run_extract_midpoints and run_inspect_parquet (lines 1923 to 2168)
func_lines = lines[1922:2168]
# Make them pub
func_lines[0] = func_lines[0].replace('async fn run_extract_midpoints', 'pub async fn run_extract_midpoints')
func_lines[145] = func_lines[145].replace('fn run_inspect_parquet', 'pub fn run_inspect_parquet')

# Also we need to get ExtractMidpointsArgs and InspectParquetArgs from bot.rs.bak
# InspectParquetArgs is 165 to 182
inspect_args_lines = lines[164:182]
# ExtractMidpointsArgs is 283 to 300
extract_args_lines = lines[282:300]

with open(discovery_path, 'a', encoding='utf-8') as f:
    f.write('\n// -- Added from Extraction ----------------------------------------------\n')
    f.writelines(inspect_args_lines)
    f.write('\n')
    f.writelines(extract_args_lines)
    f.write('\n')
    f.writelines(func_lines)
    f.write('\n')

# Now remove these structs from bot.rs
with open(bot_path, 'r', encoding='utf-8') as f:
    bot_lines = f.readlines()

import re
# We need to find and remove InspectParquetArgs and ExtractMidpointsArgs from bot.rs
new_bot_lines = []
skip = False
for line in bot_lines:
    if 'pub struct InspectParquetArgs' in line or 'pub struct ExtractMidpointsArgs' in line:
        skip = True
        # also remove the preceding #[derive(Args)]
        if new_bot_lines and '#[derive(Args)]' in new_bot_lines[-1]:
            new_bot_lines.pop()
        continue
    if skip and line.strip() == '}':
        skip = False
        continue
    if not skip:
        new_bot_lines.append(line)

# Now we need to update bot.rs imports to import these arguments from discovery
import_added = False
for i, line in enumerate(new_bot_lines):
    if 'run_extract_midpoints' in line and 'use crate::bot::discovery' in new_bot_lines[i-2]:
        # it's the import line of discovery
        new_bot_lines[i] = line.replace('run_extract_midpoints,', 'run_extract_midpoints, ExtractMidpointsArgs, InspectParquetArgs,')
        import_added = True
        break

if not import_added:
    print("Could not find the import statement to update.")

with open(bot_path, 'w', encoding='utf-8') as f:
    f.writelines(new_bot_lines)

print('Done migrating args and functions to discovery.')
