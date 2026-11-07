#!/usr/bin/env python3
"""Compare private Sheet captures across animation profiles, excluding variable text."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('on', type=Path)
    ap.add_argument('off', type=Path)
    args = ap.parse_args()
    out = args.on.resolve().parent / ('motion-comparison-' + str(time.time_ns()) + '.json')
    report = {'functional_gate': 'FAIL', 'on_report': str(args.on.resolve()), 'off_report': str(args.off.resolve()),
              'method': 'RGB pixel comparison restricted to Sheet panel; directory and status label rectangles masked',
              'cases': {}}
    try:
        on, off = [json.loads(p.read_text()) for p in (args.on, args.off)]
        assert on['functional_gate'] == off['functional_gate'] == 'PASS'
        assert on['animations'] == 'on' and off['animations'] == 'off'
        for key in ('binary_sha256', 'helper_sha256', 'html_helper_sha256', 'window_size'):
            assert on[key] == off[key], key
        def snapshot(r):
            return next(s for s in r['states'] if s['label'] == 'create-file' and s.get('open') and s.get('built'))
        first, second = snapshot(on), snapshot(off)
        assert first['rect'] == second['rect']
        for key in ('fields', 'apply', 'cancel', 'scroll', 'directory', 'status_rect', 'values', 'control_count',
                    'status', 'apply_enabled', 'field_enabled'):
            assert first['geometry'][key] == second['geometry'][key], key
        for key in ('can_apply', 'can_edit', 'error', 'dismissible'):
            assert first[key] == second[key], key
        rect = first['rect']; x, y, width, height = rect
        assert min(x, y) >= 0 and width > 0 and height > 0
        masks = [first['geometry']['directory'], first['geometry']['status_rect']]
        assert all(len(m) == 4 and m[2] > 0 and m[3] > 0 for m in masks)
        report['panel_rect'] = rect
        report['excluded_rectangles'] = [{'reason': reason, 'rect': mask} for reason, mask in
                                         zip(('fixture-specific directory text', 'variable status text'), masks)]
        def capture(r, label):
            return next(Path(p) for p in r['captures'] if Path(p).name == label + '.png')
        def pixels(path):
            output = subprocess.check_output(['magick', str(path), '-crop', f'{width}x{height}+{x}+{y}',
                                              '+repage', '-alpha', 'off', '-depth', '8', 'rgb:-'], timeout=15)
            assert len(output) == width * height * 3
            data = bytearray(output)
            for mx, my, mw, mh in masks:
                left, right = max(x, mx), min(x + width, mx + mw)
                top, bottom = max(y, my), min(y + height, my + mh)
                for row in range(top, bottom):
                    a = ((row - y) * width + left - x) * 3
                    b = ((row - y) * width + right - x) * 3
                    data[a:b] = b'\0' * (b - a)
            return data
        cases = [('on_open_start_middle', on, 'open-start', on, 'open-middle', True),
                 ('on_open_middle_settled', on, 'open-middle', on, 'open-settled', True),
                 ('off_open_start_settled', off, 'open-start', off, 'open-settled', False),
                 ('on_off_open_settled', on, 'open-settled', off, 'open-settled', False),
                 ('on_cover_to_return_middle', on, 'cover-middle', on, 'cover-return-middle', True),
                 ('on_return_middle_settled', on, 'cover-return-middle', on, 'cover-return-settled', True)]
        for label, ar, an, br, bn, changed in cases:
            a, b = capture(ar, an), capture(br, bn)
            pa, pb = pixels(a), pixels(b)
            count = sum(pa[i:i+3] != pb[i:i+3] for i in range(0, len(pa), 3))
            report['cases'][label] = {'changed_pixels': count, 'images': [str(a), str(b)],
                                      'sha256': [hashlib.sha256(p.read_bytes()).hexdigest() for p in (a, b)]}
            assert (count > 0) == changed, label
        report['functional_gate'] = 'PASS'
    except Exception as exc:
        report['error'] = repr(exc)
    out.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'functional_gate': report['functional_gate'], 'report': str(out), 'error': report.get('error')}))
    return 0 if report['functional_gate'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
