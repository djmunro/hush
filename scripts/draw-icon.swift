// Draws the hush app icon as a 1024×1024 PNG. Same visual language as
// the menubar template icon (mic + sound waves), but full-color and at
// app-icon scale.
//
// Headless-safe: uses NSBitmapImageRep + NSGraphicsContext directly
// instead of NSImage.lockFocus (which needs a real graphics environment
// and silently produces garbage in `swift script` mode).
//
// Usage:  swift draw-icon.swift <output-path.png>

import AppKit
import CoreGraphics

guard CommandLine.arguments.count == 2 else {
    FileHandle.standardError.write("usage: draw-icon.swift <output.png>\n".data(using: .utf8)!)
    exit(1)
}
let outPath = CommandLine.arguments[1]

let pixelSize = 1024
guard let rep = NSBitmapImageRep(
    bitmapDataPlanes: nil,
    pixelsWide: pixelSize,
    pixelsHigh: pixelSize,
    bitsPerSample: 8,
    samplesPerPixel: 4,
    hasAlpha: true,
    isPlanar: false,
    colorSpaceName: .deviceRGB,
    bytesPerRow: 0,
    bitsPerPixel: 32
) else {
    FileHandle.standardError.write("failed to allocate bitmap\n".data(using: .utf8)!)
    exit(1)
}

guard let ctx = NSGraphicsContext(bitmapImageRep: rep) else {
    FileHandle.standardError.write("failed to create graphics context\n".data(using: .utf8)!)
    exit(1)
}

NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = ctx

let size = CGFloat(pixelSize)

// Rounded-square background with a soft purple gradient.
let bgRect = NSRect(x: 0, y: 0, width: size, height: size)
let bgPath = NSBezierPath(roundedRect: bgRect, xRadius: 220, yRadius: 220)
if let bg = NSGradient(colorsAndLocations:
    (NSColor(calibratedRed: 0.32, green: 0.22, blue: 0.62, alpha: 1.0), 0.0),
    (NSColor(calibratedRed: 0.10, green: 0.08, blue: 0.28, alpha: 1.0), 1.0)) {
    bg.draw(in: bgPath, angle: 90)
}

NSColor.white.setFill()
NSColor.white.setStroke()

// Microphone capsule.
let capsuleW: CGFloat = 280
let capsuleH: CGFloat = 480
let capX = (size - capsuleW) / 2 - 60
let capY = (size - capsuleH) / 2 + 60
let capsuleRect = NSRect(x: capX, y: capY, width: capsuleW, height: capsuleH)
NSBezierPath(roundedRect: capsuleRect, xRadius: capsuleW / 2, yRadius: capsuleW / 2).fill()

// Stand: arc + stem + base.
let cx = capX + capsuleW / 2
let arcRadius: CGFloat = 230
let stroke: CGFloat = 38

let arc = NSBezierPath()
arc.lineWidth = stroke
arc.lineCapStyle = .round
arc.appendArc(withCenter: NSPoint(x: cx, y: capY),
              radius: arcRadius,
              startAngle: 180,
              endAngle: 360)
arc.stroke()

let stem = NSBezierPath()
stem.lineWidth = stroke
stem.lineCapStyle = .round
stem.move(to: NSPoint(x: cx, y: capY - arcRadius))
stem.line(to: NSPoint(x: cx, y: capY - arcRadius - 100))
stem.stroke()

let base = NSBezierPath()
base.lineWidth = stroke
base.lineCapStyle = .round
base.move(to: NSPoint(x: cx - 110, y: capY - arcRadius - 100))
base.line(to: NSPoint(x: cx + 110, y: capY - arcRadius - 100))
base.stroke()

// Sound waves: three ticks (short, tall, short) — implies "shh" cadence.
let waveX = capX + capsuleW + 90
let waveCY = capY + capsuleH / 2
let waveStroke: CGFloat = 32
let halves: [CGFloat] = [70, 100, 70]
for (i, h) in halves.enumerated() {
    let dx = waveX + CGFloat(i) * 70
    let line = NSBezierPath()
    line.lineWidth = waveStroke
    line.lineCapStyle = .round
    line.move(to: NSPoint(x: dx, y: waveCY - h))
    line.line(to: NSPoint(x: dx, y: waveCY + h))
    line.stroke()
}

NSGraphicsContext.restoreGraphicsState()

guard let png = rep.representation(using: .png, properties: [:]) else {
    FileHandle.standardError.write("failed to encode PNG\n".data(using: .utf8)!)
    exit(1)
}

try png.write(to: URL(fileURLWithPath: outPath))
print("wrote \(outPath)")
