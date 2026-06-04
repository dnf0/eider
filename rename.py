import os
import subprocess

def replace_in_file(filepath):
    try:
        with open(filepath, 'r', encoding='utf-8') as f:
            content = f.read()
    except Exception:
        return False

    new_content = content.replace('zarrduck', 'eider').replace('Zarrduck', 'Eider').replace('ZARRDUCK', 'EIDER')
    if new_content != content:
        with open(filepath, 'w', encoding='utf-8') as f:
            f.write(new_content)
        return True
    return False

output = subprocess.check_output(['git', 'ls-files']).decode('utf-8')
files = output.strip().split('\n')

for file in files:
    if os.path.isfile(file):
        if replace_in_file(file):
            print(f"Updated {file}")
