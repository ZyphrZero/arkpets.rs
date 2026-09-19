// Regenerate the original ArkPets Dock icon: swift docs/scripts/Generate-MacOSIcon.swift
import AppKit

let image = NSImage(size: NSSize(width: 512, height: 512))
image.lockFocus()
NSColor(srgbRed: 0, green: 126.0 / 255, blue: 111.0 / 255, alpha: 1).setFill()
NSBezierPath(roundedRect: NSRect(x: 24, y: 24, width: 464, height: 464), xRadius: 108, yRadius: 108).fill()
NSColor.white.setFill()
for rect in [
    NSRect(x: 107, y: 255, width: 63, height: 87),
    NSRect(x: 179, y: 313, width: 66, height: 92),
    NSRect(x: 267, y: 313, width: 66, height: 92),
    NSRect(x: 342, y: 255, width: 63, height: 87),
] {
    NSBezierPath(ovalIn: rect).fill()
}
let pad = NSBezierPath()
pad.move(to: NSPoint(x: 149, y: 158))
pad.curve(to: NSPoint(x: 211, y: 242), controlPoint1: NSPoint(x: 139, y: 192), controlPoint2: NSPoint(x: 183, y: 213))
pad.curve(to: NSPoint(x: 301, y: 242), controlPoint1: NSPoint(x: 235, y: 281), controlPoint2: NSPoint(x: 277, y: 281))
pad.curve(to: NSPoint(x: 363, y: 158), controlPoint1: NSPoint(x: 329, y: 213), controlPoint2: NSPoint(x: 373, y: 192))
pad.curve(to: NSPoint(x: 256, y: 151), controlPoint1: NSPoint(x: 348, y: 109), controlPoint2: NSPoint(x: 291, y: 138))
pad.curve(to: NSPoint(x: 149, y: 158), controlPoint1: NSPoint(x: 221, y: 138), controlPoint2: NSPoint(x: 164, y: 109))
pad.close()
pad.fill()
image.unlockFocus()
let bitmap = NSBitmapImageRep(data: image.tiffRepresentation!)!
let png = bitmap.representation(using: .png, properties: [:])!
let path = URL(fileURLWithPath: "assets/arkpets-icon.png")
try png.write(to: path)
print("Wrote \(path.path)")
