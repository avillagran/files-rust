#!/usr/bin/env python3
"""Compare the settled Preferences control in two retained private runs."""
import json
import subprocess
import sys
from pathlib import Path


def load(path):
    report = json.loads(Path(path).read_text())
    if report.get('profile_gate') != 'PASS':
        raise ValueError('profile did not pass')
    state = next(item for item in report['states'] if item['label'] == 'hover-settled')
    rect = state['toolbar']['items'][-1]['rect']
    image = next(Path(item) for item in report['captures'] if item.endswith('/hover-settled.png'))
    width, height = map(int, subprocess.check_output(
        ['magick', 'identify', '-format', '%w %h', str(image)], text=True).split())
    if not (rect[0] >= 0 and rect[1] >= 0 and rect[0] + rect[2] <= width and rect[1] + rect[3] <= height):
        raise ValueError('control lies outside captured frame')
    raw = subprocess.check_output(['magick', str(image), '-depth', '8', 'rgb:-'])
    if len(raw) != width * height * 3:
        raise ValueError('invalid image bytes')
    x, y, w, h = rect
    pixels = b''.join(raw[3 * ((y + row) * width + x):3 * ((y + row) * width + x + w)] for row in range(h))
    return report, rect, pixels


def main():
    if len(sys.argv) != 3:
        raise SystemExit('usage: compare-compact-motion.py ON.json OFF.json')
    on, on_rect, on_pixels = load(sys.argv[1])
    off, off_rect, off_pixels = load(sys.argv[2])
    if on['animations'] != 'on' or off['animations'] != 'off' or on['requested_size'] != off['requested_size']:
        raise ValueError('expected matching ON and OFF profiles')
    changed = sum(a != b for a, b in zip(on_pixels, off_pixels)) if on_rect == off_rect else None
    result = {'scope': 'settled Preferences control only', 'size': on['requested_size'],
              'on_rect': on_rect, 'off_rect': off_rect, 'different_rgb_channels': changed,
              'on_early_width': on.get('hover_early_width'), 'on_early_animating': on.get('hover_early_animating'),
              'off_early_width': off.get('hover_early_width'), 'off_early_animating': off.get('hover_early_animating')}
    result['motion_observed'] = (on.get('hover_early_animating') is True and
                                 36 <= on.get('hover_early_width', 0) < on_rect[2] and
                                 off.get('hover_early_animating') is False and
                                 off.get('hover_early_width') == off_rect[2])
    print(json.dumps(result, indent=2))
    return 0 if changed == 0 and result['motion_observed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
