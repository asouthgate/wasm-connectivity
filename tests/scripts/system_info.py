#!/usr/bin/env python3
"""Dump system info for benchmark documentation."""
import os, subprocess, platform

def read_first(path):
    try:
        with open(path) as f:
            return f.readline().strip()
    except Exception:
        return 'N/A'

def run(cmd):
    try:
        return subprocess.run(cmd, capture_output=True, text=True).stdout.strip()
    except Exception:
        return 'N/A'

cpu_model = 'N/A'
for line in run(['lscpu']).split('\n'):
    if 'Model name' in line:
        cpu_model = line.split(':')[-1].strip()
        break

mem_total = read_first('/proc/meminfo')
ram_gb = 'N/A'
if 'MemTotal' in mem_total:
    ram_gb = f"{int(mem_total.split()[1]) / (1024*1024):.1f} GB"

chrome_ver = run(['google-chrome', '--version']) or run(['google-chrome-stable', '--version']) or run(['chromium', '--version']) or run(['chromium-browser', '--version']) or 'N/A'

print(f'OS:        {platform.system()} {platform.release()}')
print(f'CPU:       {cpu_model}')
print(f'Cores:     {os.cpu_count()}')
print(f'RAM:       {ram_gb}')
print(f'Chrome:    {chrome_ver}')
