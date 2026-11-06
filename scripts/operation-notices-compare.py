#!/usr/bin/env python3
"""Compare private ON/OFF notice captures, limited to the painted card."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('on', type=Path)
    parser.add_argument('off', type=Path)
    args = parser.parse_args()
    result = {'functional_gate': 'FAIL', 'on_report': str(args.on.resolve()), 'off_report': str(args.off.resolve()),
              'method': 'ImageMagick AE restricted to notice rectangle; excludes variable browser status pixels',
              'cases': {}}
    target = args.on.resolve().parent / ('pixel-comparison-' + str(time.time_ns()) + '.json')
    try:
        on, off = [json.loads(p.read_text()) for p in (args.on, args.off)]
        assert on['functional_gate'] == off['functional_gate'] == 'PASS'
        assert on['animations'] == 'on' and off['animations'] == 'off'
        for key in ('binary_sha256', 'helper_sha256', 'html_helper_sha256', 'window_size'):
            assert on[key] == off[key], (key, on[key], off[key])
        def state(report):
            return next(s for s in report['states'] if s['label'] == 'info')
        a, b = state(on), state(off)
        for key in ('width', 'height', 'browser_focus', 'selection_raw', 'selected_raw', 'anchor_raw'):
            assert a[key] == b[key], (key, a[key], b[key])
        for key in ('rect', 'source', 'text', 'visible'):
            assert a['card'][key] == b['card'][key], (key, a['card'][key], b['card'][key])
        x, y, w, h = a['card']['rect']
        assert x >= 0 and y >= 0 and w > 0 and h > 0 and x + w <= a['width'] and y + h <= a['height']
        crop = f'{w}x{h}+{x}+{y}'
        result['crop'] = crop
        def image(report, name):
            return next(Path(p) for p in report['captures'] if Path(p).name == name + '.png')
        cases = [('on_start_vs_middle', image(on, 'info-start'), image(on, 'info-middle')),
                 ('on_middle_vs_settled', image(on, 'info-middle'), image(on, 'info-settled')),
                 ('off_start_vs_settled', image(off, 'info-start'), image(off, 'info-settled')),
                 ('on_vs_off_settled', image(on, 'info-settled'), image(off, 'info-settled'))]
        for label, first, second in cases:
            command = ['magick', 'compare', '-metric', 'AE', '(', str(first), '-crop', crop, '+repage', ')',
                       '(', str(second), '-crop', crop, '+repage', ')', 'null:']
            process = subprocess.run(command, capture_output=True, text=True, timeout=15)
            assert process.returncode in (0, 1), process.stderr
            result['cases'][label] = {'ae': float(process.stderr.split()[0]), 'images': [str(first), str(second)],
                                      'sha256': [hashlib.sha256(p.read_bytes()).hexdigest() for p in (first, second)]}
        for label, expected_change in (('on_start_vs_middle', True), ('on_middle_vs_settled', True),
                                       ('off_start_vs_settled', False), ('on_vs_off_settled', False)):
            assert (result['cases'][label]['ae'] > 0) == expected_change, (label, result['cases'][label])
        result['functional_gate'] = 'PASS'
    except Exception as exc:
        result['error'] = repr(exc)
    target.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'report': str(target), 'functional_gate': result['functional_gate'], 'error': result.get('error')}))
    return 0 if result['functional_gate'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
