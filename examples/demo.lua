-- demo.lua — Photonic Lua scripting demo
-- Generates a geometric composition and saves it as a PNG.

local p = photonic

print("Canvas: " .. p.width() .. " x " .. p.height())
p.clear()

-- ── Background gradient simulation (concentric rects) ─────────────────────────
local bg_colors = {
    p.color.hsl(0.62, 0.6, 0.12),
    p.color.hsl(0.62, 0.6, 0.16),
    p.color.hsl(0.62, 0.6, 0.20),
}
local W, H = p.width(), p.height()
for i, col in ipairs(bg_colors) do
    local pad = (i - 1) * 60
    p.create_rect(pad, pad, W - pad * 2, H - pad * 2, {
        fill = col, name = "bg_" .. i
    })
end

-- ── Grid of coloured circles ──────────────────────────────────────────────────
local cols, rows = 7, 5
local margin = 80
local cell_w = (W - margin * 2) / cols
local cell_h = (H - margin * 2) / rows

for row = 0, rows - 1 do
    for col = 0, cols - 1 do
        local cx = margin + col * cell_w + cell_w / 2
        local cy = margin + row * cell_h + cell_h / 2
        local r  = math.min(cell_w, cell_h) * 0.38

        -- Hue varies across the grid
        local hue = (row * cols + col) / (rows * cols)
        local fill = p.color.hsv(hue, 0.75, 0.95)

        p.create_circle(cx, cy, r, {
            fill = fill,
            name = string.format("circle_%d_%d", row, col),
        })
    end
end

-- ── Central star ─────────────────────────────────────────────────────────────
p.create_star(W / 2, H / 2, 110, 48, 8, {
    fill  = "#FFFFFF",
    name  = "star_center",
    opacity = 0.92,
})

-- ── Corner polygons ───────────────────────────────────────────────────────────
local corners = {
    { W * 0.12, H * 0.12 },
    { W * 0.88, H * 0.12 },
    { W * 0.12, H * 0.88 },
    { W * 0.88, H * 0.88 },
}
for i, pos in ipairs(corners) do
    local hue = (i - 1) / 4
    p.create_polygon(pos[1], pos[2], 55, 6, {
        fill = p.color.hsv(hue, 0.6, 1.0),
        name = "hex_corner_" .. i,
    })
end

print("Nodes created: " .. p.node_count())

-- ── Save ─────────────────────────────────────────────────────────────────────
p.save("demo_output.png")
print("Done!")
