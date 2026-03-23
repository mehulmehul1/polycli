import sys
import shutil

file_path = r'c:\Users\mehul\OneDrive\Desktop\Studio\PROJECTS\polycli\polycli\src\commands\bot.rs'
shutil.copy2(file_path, file_path + '.bak')

with open(file_path, 'r', encoding='utf-8') as f:
    lines = f.readlines()

new_lines = []
for i, line in enumerate(lines, 1):
    # exact line ranges verified using view_file
    if (1274 <= i <= 1570) or (1798 <= i <= 2673) or (3228 <= i <= 3278):
        continue
    new_lines.append(line)

with open(file_path, 'w', encoding='utf-8') as f:
    f.writelines(new_lines)

print('Lines removed successfully.')
