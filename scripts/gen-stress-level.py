#!/usr/bin/env python3
"""Generates assets/levels/stress_lights.json, a lighting benchmark scene.

Six stacked galleries around one open atrium, with a configurable number of
lit sconces spread across them. The point is to see many levels' worth of
lights at once, above and below, which is what a grand-scene lighting budget
actually has to survive.

The geometry exists to satisfy two rules in dungeon.rs: a cell renders a floor
only when the layer below it is solid, and a ceiling only when the layer above
is. So the galleries alternate between two rings — even layers take the inner
ring, odd layers the outer — which leaves rock under and over every walkable
ledge while the atrium, open on every layer, stays a clear shaft from the
bottom floor to the top ceiling.

Regenerate with: python3 scripts/gen-stress-level.py
"""

import json
from pathlib import Path

GRID = 48
LAYERS = 6
SCONCES_PER_LAYER = 50

# Atrium: open on every layer, so it renders neither floor nor ceiling and you
# can see straight through the whole stack.
ATRIUM_MIN, ATRIUM_MAX = 16, 31

# The two gallery rings, as inclusive distances outward from the atrium edge.
# They must not overlap, or a gallery would sit above open space and lose its
# floor.
INNER_RING = (1, 3)
OUTER_RING = (4, 6)

ROCK, FLOOR = "#", "."


def ring_bounds(ring):
    near, far = ring
    return ATRIUM_MIN - far, ATRIUM_MIN - near, ATRIUM_MAX + near, ATRIUM_MAX + far


def in_atrium(col, row):
    return ATRIUM_MIN <= col <= ATRIUM_MAX and ATRIUM_MIN <= row <= ATRIUM_MAX


def in_ring(col, row, ring):
    outer_low, inner_low, inner_high, outer_high = ring_bounds(ring)
    inside_outer = outer_low <= col <= outer_high and outer_low <= row <= outer_high
    inside_inner = inner_low < col < inner_high and inner_low < row < inner_high
    return inside_outer and not inside_inner


def build_grid(ring):
    rows = []
    for row in range(GRID):
        cells = []
        for col in range(GRID):
            walkable = in_atrium(col, row) or in_ring(col, row, ring)
            cells.append(FLOOR if walkable else ROCK)
        rows.append("".join(cells))
    return rows


def gallery_cells(ring):
    """Ring cells in a stable ring order, so sconces spread evenly around it."""
    outer_low, _, _, outer_high = ring_bounds(ring)
    top = [(col, outer_low) for col in range(outer_low, outer_high + 1)]
    right = [(outer_high, row) for row in range(outer_low + 1, outer_high + 1)]
    bottom = [(col, outer_high) for col in range(outer_high - 1, outer_low - 1, -1)]
    left = [(outer_low, row) for row in range(outer_high - 1, outer_low, -1)]
    return top + right + bottom + left


def mounting_wall(col, row, ring):
    """The outward wall, so the sconce faces in toward the atrium."""
    outer_low, _, _, outer_high = ring_bounds(ring)
    distances = {
        "N": row - outer_low,
        "S": outer_high - row,
        "W": col - outer_low,
        "E": outer_high - col,
    }
    return min(distances, key=distances.get)


def build_layer(index):
    ring = INNER_RING if index % 2 == 0 else OUTER_RING
    cells = gallery_cells(ring)
    step = max(1, len(cells) // SCONCES_PER_LAYER)
    entities = []
    for count, cell_index in enumerate(range(0, len(cells), step)):
        if count >= SCONCES_PER_LAYER:
            break
        col, row = cells[cell_index]
        entities.append(
            {
                "id": f"sconce_l{index}_{count:02d}",
                "col": col,
                "row": row,
                "type": "torch_sconce",
                "wall": mounting_wall(col, row, ring),
                "lit": True,
            }
        )
    return {"id": str(index), "grid": build_grid(ring), "entities": entities}


def main():
    layers = [build_layer(index) for index in range(LAYERS)]
    total_lights = sum(len(layer["entities"]) for layer in layers)
    # Stand on a middle gallery, facing the atrium, with layers both above and
    # below in view.
    start_ring = INNER_RING if 2 % 2 == 0 else OUTER_RING
    start_col = ring_bounds(start_ring)[0]
    dungeon = {
        "name": "Lighting Stress Test",
        "playerStart": {
            "levelId": "stress",
            "col": start_col,
            "row": GRID // 2,
            "facing": "E",
            "layerIndex": 2,
        },
        "levels": [
            {
                "id": "stress",
                "name": "Grand Atrium",
                "environment": "dungeon",
                "layers": layers,
            }
        ],
    }

    out = Path(__file__).resolve().parent.parent / "assets" / "levels" / "stress_lights.json"
    out.write_text(json.dumps(dungeon, indent=4) + "\n")
    print(f"wrote {out} — {LAYERS} layers, {total_lights} lit sconces")


if __name__ == "__main__":
    main()
