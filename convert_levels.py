#!/usr/bin/env python3
"""
Convert Lode Runner binary level data (levels.txt C arrays) to NodeRunner text format.

Based on the Level Array Format Specification v1.0:
- Header: player pos, enemies, exit ladders, respawn points, encryption type
- Map body: GRID (type 2), Row RLE (type 0), or Column RLE (type 1)
- Tile IDs: 0=Blank, 1=Brick, 2=Solid, 3=Ladder, 4=Rail, 5=Fall-through, 6=Gold
"""

import re
import sys
import os

# Tile ID -> NodeRunner character mapping
TILE_CHARS = {
    0: ' ',  # Blank  -> Empty
    1: '#',  # Brick  -> Brick (diggable)
    2: '=',  # Solid  -> Concrete (indestructible)
    3: 'H',  # Ladder -> Ladder
    4: '-',  # Rail   -> Rope
    5: 'T',  # Fall-through -> TrapBrick
    6: '$',  # Gold   -> Gold
}

WIDTH = 28
HEIGHT = 16


def parse_c_arrays(filepath):
    """Parse levels.txt and extract named byte arrays."""
    with open(filepath, 'r') as f:
        content = f.read()

    # Match each array: const uint8_t PROGMEM name[] = { ... };
    pattern = r'const\s+uint8_t\s+PROGMEM\s+(\w+)\[\]\s*=\s*\{([^}]+)\};'
    levels = []
    for m in re.finditer(pattern, content):
        name = m.group(1)
        hex_str = m.group(2)
        # Parse hex values
        bytes_list = []
        for h in re.findall(r'0x([0-9A-Fa-f]{1,2})', hex_str):
            bytes_list.append(int(h, 16))
        levels.append((name, bytes_list))
    return levels


def decode_level(name, data):
    """Decode a binary level array into a 28x16 grid + entity positions."""
    pos = 0

    def read_byte():
        nonlocal pos
        if pos >= len(data):
            raise ValueError(f"Unexpected end of data at pos {pos} in {name}")
        b = data[pos]
        pos += 1
        return b

    # --- Header ---
    # 5.1 Player start position
    player_x = read_byte()
    player_y = read_byte()

    # 5.2 Enemy info
    enemy_count = read_byte()
    enemies = []
    for _ in range(enemy_count):
        ex = read_byte()
        ey = read_byte()
        enemies.append((ex, ey))

    # 5.3 Exit ladders (appear after collecting all gold)
    ladder_count = read_byte()
    exit_ladders = []
    for _ in range(ladder_count):
        lx = read_byte()
        ly = read_byte()
        exit_ladders.append((lx, ly))

    # 5.4 Respawn points (usually 4, but some levels differ)
    # 5.5 Encryption type
    # Auto-detect: try standard 4 respawns first, fallback to other counts
    saved_pos = pos
    respawn_points = []
    encryption_type = None

    for try_rc in [4, 0, 1, 2, 3, 5, 6]:
        pos = saved_pos
        rp = []
        valid = True
        for _ in range(try_rc):
            if pos + 1 >= len(data):
                valid = False
                break
            rp.append((data[pos], data[pos + 1]))
            pos += 2
        if not valid or pos >= len(data):
            continue
        enc = data[pos]
        if enc not in (0, 1, 2):
            continue
        # Verify RLE decodes to exactly 448 tiles
        test_pos = pos + 1
        cursor = 0
        ok = True
        if enc in (0, 1):
            while cursor < WIDTH * HEIGHT and test_pos < len(data):
                b = data[test_pos]
                tile = (b >> 5) & 0x07
                run = b & 0x1F
                if run == 0:
                    break
                if tile > 6:
                    ok = False
                    break
                cursor += run
                test_pos += 1
            if cursor != WIDTH * HEIGHT:
                ok = False
        # GRID: just check remaining bytes >= 224
        elif enc == 2:
            if len(data) - test_pos < 224:
                ok = False

        if ok:
            respawn_points = rp
            encryption_type = enc
            pos += 1  # skip encryption type byte
            break

    if encryption_type is None:
        print(f"  WARNING: Could not determine header format for {name}", file=sys.stderr)
        return None

    # --- Map Body ---
    grid = [[0] * WIDTH for _ in range(HEIGHT)]

    if encryption_type == 2:
        # GRID: 224 bytes, nibble-packed
        for y in range(HEIGHT):
            for x_byte in range(14):  # 14 bytes per row
                b = read_byte()
                tile_even = (b >> 4) & 0x0F
                tile_odd = b & 0x0F
                x = x_byte * 2
                if tile_even <= 6:
                    grid[y][x] = tile_even
                if x + 1 < WIDTH and tile_odd <= 6:
                    grid[y][x + 1] = tile_odd

    elif encryption_type == 0 or encryption_type == 1:
        # RLE: bbb rrrrr (upper 3 bits = tile, lower 5 bits = run)
        cursor = 0
        while cursor < WIDTH * HEIGHT:
            if pos >= len(data):
                break
            b = read_byte()
            tile = (b >> 5) & 0x07
            run = b & 0x1F
            if run == 0:
                break  # Terminator
            for _ in range(run):
                if cursor >= WIDTH * HEIGHT:
                    break
                if encryption_type == 0:
                    # Row RLE: left->right, top->bottom
                    x = cursor % WIDTH
                    y = cursor // WIDTH
                else:
                    # Column RLE: top->bottom, left->right
                    x = cursor // HEIGHT
                    y = cursor % HEIGHT
                if 0 <= x < WIDTH and 0 <= y < HEIGHT and tile <= 6:
                    grid[y][x] = tile
                cursor += 1
    else:
        print(f"  WARNING: Unknown encryption type {encryption_type} for {name}", file=sys.stderr)
        return None

    # --- Build text rows ---
    rows = []
    for y in range(HEIGHT):
        row_chars = list(' ' * WIDTH)
        for x in range(WIDTH):
            tile_id = grid[y][x]
            row_chars[x] = TILE_CHARS.get(tile_id, ' ')
        rows.append(row_chars)

    # Overlay entities onto grid
    # Player
    if 0 <= player_x < WIDTH and 0 <= player_y < HEIGHT:
        rows[player_y][player_x] = 'P'

    # Enemies
    for (ex, ey) in enemies:
        if 0 <= ex < WIDTH and 0 <= ey < HEIGHT:
            rows[ey][ex] = 'E'

    # Exit ladders -> place '~' at exact positions, track overlaps
    overlap_positions = []
    for (lx, ly) in exit_ladders:
        if 0 <= lx < WIDTH and 0 <= ly < HEIGHT:
            if rows[ly][lx] == ' ':
                rows[ly][lx] = '~'
            else:
                # Position overlaps with another tile (e.g. gold)
                overlap_positions.append((lx, ly))

    # Convert to strings
    text_rows = [''.join(r) for r in rows]

    return {
        'name': name,
        'rows': text_rows,
        'player': (player_x, player_y),
        'enemies': enemies,
        'exit_ladders': exit_ladders,
        'overlap_positions': overlap_positions,
        'respawn_points': respawn_points,
        'encryption_type': encryption_type,
    }


def level_to_text(level_data, index):
    """Convert decoded level to NodeRunner text file format."""
    name = level_data['name']

    # Generate a readable level name
    if name == 'test':
        display_name = "Test Level"
    else:
        # Extract number from 'levelN'
        m = re.match(r'level(\d+)', name)
        if m:
            num = int(m.group(1))
            display_name = f"Level {num}"
        else:
            display_name = name

    lines = [f"# {display_name}"]

    # Add metadata for hidden ladder positions that overlap with other tiles
    overlap = level_data.get('overlap_positions', [])
    if overlap:
        coords = ' '.join(f'{x},{y}' for x, y in overlap)
        lines.append(f'@ {coords}')

    for row in level_data['rows']:
        lines.append(row)

    return '\n'.join(lines) + '\n'


def main():
    input_file = sys.argv[1] if len(sys.argv) > 1 else '/mnt/user-data/uploads/levels.txt'
    output_dir = sys.argv[2] if len(sys.argv) > 2 else '/home/claude/noderunner/loderunner/levels'

    print(f"Reading: {input_file}")
    print(f"Output:  {output_dir}")

    levels = parse_c_arrays(input_file)
    print(f"Found {len(levels)} level arrays")

    os.makedirs(output_dir, exist_ok=True)

    # Remove existing level files
    for f in os.listdir(output_dir):
        if f.endswith('.txt'):
            os.remove(os.path.join(output_dir, f))

    success = 0
    errors = 0
    level_index = 0

    for name, data in levels:
        try:
            decoded = decode_level(name, data)
            if decoded is None:
                errors += 1
                continue

            # Generate filename with zero-padded index
            if name == 'test':
                filename = f"000_test.txt"
            else:
                m = re.match(r'level(\d+)', name)
                if m:
                    num = int(m.group(1))
                    filename = f"{num:03d}_level{num}.txt"
                else:
                    filename = f"{level_index:03d}_{name}.txt"

            text = level_to_text(decoded, level_index)
            filepath = os.path.join(output_dir, filename)
            with open(filepath, 'w') as f:
                f.write(text)

            success += 1
            level_index += 1

            # Debug: print first few levels
            if level_index <= 3:
                print(f"\n=== {name} (encryption={decoded['encryption_type']}) ===")
                print(f"Player: {decoded['player']}")
                print(f"Enemies: {decoded['enemies']}")
                print(f"Exit ladders: {decoded['exit_ladders']}")
                for row in decoded['rows']:
                    print(f"|{row}|")

        except Exception as e:
            print(f"ERROR processing {name}: {e}", file=sys.stderr)
            import traceback
            traceback.print_exc()
            errors += 1

    print(f"\nDone: {success} levels converted, {errors} errors")


if __name__ == '__main__':
    main()
