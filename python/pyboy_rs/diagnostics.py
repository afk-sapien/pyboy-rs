"""Optional checkpoint diagnostics for deterministic input recordings."""

from copy import deepcopy
import hashlib


class ReplayDivergence(RuntimeError):
    def __init__(self, details):
        self.details = deepcopy(details)
        super().__init__(f"Replay diverged at checkpoint {details['first_failed_offset']} "
                         f"after verified offset {details['last_verified_offset']}")


def validate_ranges(ranges):
    result = []
    for start, stop in ranges:
        if type(start) is not int or type(stop) is not int or not 0xc000 <= start < stop <= 0xe000:
            raise ValueError('Diagnostic ranges must be nonempty WRAM ranges within C000:E000')
        if any(start < other_stop and other_start < stop for other_start, other_stop in result):
            raise ValueError('Diagnostic ranges must not overlap')
        result.append((start, stop))
    return result


def snapshot(emulator, offset, ranges):
    return {'offset': offset, 'frame': emulator.frame_count,
            'state_sha256': hashlib.sha256(emulator._machine.save_state()).hexdigest(),
            'runtime_sha256': hashlib.sha256(emulator._machine.execution_runtime()).hexdigest(),
            'overlays_sha256': hashlib.sha256(repr([(bank, address, emulator.memory[bank, address])
                for (bank, address), original in sorted(emulator.memory._rom_original.items())
                if emulator.memory[bank, address] != original]).encode()).hexdigest(),
            'memory': [(start, emulator.memory.read_bytes(start, stop)) for start, stop in ranges]}


def validate_checkpoints(recording):
    checkpoints = recording.get('checkpoints', [])
    ranges = validate_ranges(recording.get('diagnostic_ranges', []))
    offsets = [point['offset'] for point in checkpoints]
    count = len(recording['frames'])
    if len(checkpoints) > 4096 or any(type(offset) is not int or not 0 < offset <= count for offset in offsets):
        raise ValueError('Invalid diagnostic checkpoint offset')
    if offsets != sorted(set(offsets)):
        raise ValueError('Diagnostic checkpoints must be ordered and unique')
    for point in checkpoints:
        if point['frame'] != recording['initial']['frame'] + point['offset']:
            raise ValueError('Diagnostic checkpoint frame mismatch')
        if [(start, start + len(data)) for start, data in point['memory']] != ranges:
            raise ValueError('Diagnostic memory does not match its ranges')
        if any(not isinstance(data, bytes) for _, data in point['memory']):
            raise ValueError('Diagnostic memory must contain bytes')
        for key in ('state_sha256', 'runtime_sha256', 'overlays_sha256'):
            value = point[key]
            if not isinstance(value, str) or len(value) != 64 or any(c not in '0123456789abcdef' for c in value):
                raise ValueError('Invalid diagnostic checksum')
    return {point['offset']: point for point in checkpoints}, ranges


def compare(recording, expected, actual, last_verified):
    if (expected['state_sha256'] == actual['state_sha256']
            and expected['runtime_sha256'] == actual['runtime_sha256']
            and expected['overlays_sha256'] == actual['overlays_sha256']
            and expected['memory'] == actual['memory']):
        return
    differences = []
    count = 0
    for (start, before), (_, after) in zip(expected['memory'], actual['memory']):
        for index, (old, new) in enumerate(zip(before, after)):
            if old != new:
                count += 1
                if len(differences) < 64:
                    differences.append({'address': start + index, 'expected': old, 'actual': new})
    offset = expected['offset']
    context_start = max(0, offset - 4)
    details = {'last_verified_offset': last_verified, 'first_failed_offset': offset,
               'absolute_frame': actual['frame'], 'resolution_frames': offset - last_verified,
               'expected_state_sha256': expected['state_sha256'], 'actual_state_sha256': actual['state_sha256'],
               'expected_runtime_sha256': expected['runtime_sha256'], 'actual_runtime_sha256': actual['runtime_sha256'],
               'expected_overlays_sha256': expected['overlays_sha256'], 'actual_overlays_sha256': actual['overlays_sha256'],
               'memory_differences': differences, 'memory_difference_count': count,
               'memory_differences_truncated': count > 64,
               'nearby_inputs': [{'offset': i + 1, **recording['frames'][i]}
                                 for i in range(context_start, min(len(recording['frames']), offset + 3))]}
    raise ReplayDivergence(details)
