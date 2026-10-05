// Draws the Dosi Tracker app icon — the brand mark (a white ring and dot) on a
// brand-blue rounded tile — into an .iconset folder for `iconutil -c icns`.
// Drawn in code, like the Windows agent's tray icon, so the repository holds
// no binary artwork. Run by scripts/build-app.sh:
//
//     swift scripts/make-icon.swift <output.iconset>

import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("make-icon: \(message)\n".utf8))
    exit(1)
}

/// The icon at `pixels` × `pixels`.
func drawIcon(pixels: Int) -> CGImage? {
    let size = CGFloat(pixels)
    guard let space = CGColorSpace(name: CGColorSpace.sRGB),
          let context = CGContext(data: nil, width: pixels, height: pixels, bitsPerComponent: 8, bytesPerRow: 0,
                                  space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
    else {
        return nil
    }
    // The macOS icon grid: the tile fills about 80% of the canvas, with a
    // continuous-looking corner of about 22% of its side and a soft shadow.
    let inset = size * 0.1
    let tile = CGRect(x: inset, y: inset * 1.15, width: size - 2 * inset, height: size - 2 * inset)
    let radius = tile.width * 0.225
    context.setShadow(offset: CGSize(width: 0, height: -size * 0.012), blur: size * 0.03,
                      color: CGColor(srgbRed: 0, green: 0, blue: 0, alpha: 0.28))
    context.addPath(CGPath(roundedRect: tile, cornerWidth: radius, cornerHeight: radius, transform: nil))
    context.setFillColor(CGColor(srgbRed: 0.0, green: 0.42, blue: 1.0, alpha: 1))
    context.fillPath()
    context.setShadow(offset: .zero, blur: 0, color: nil)

    let white = CGColor(srgbRed: 1, green: 1, blue: 1, alpha: 1)
    let center = CGPoint(x: tile.midX, y: tile.midY)
    context.setStrokeColor(white)
    context.setLineWidth(tile.width * 0.075)
    context.addArc(center: center, radius: tile.width * 0.27, startAngle: 0, endAngle: .pi * 2, clockwise: false)
    context.strokePath()
    let dot = tile.width * 0.11
    context.setFillColor(white)
    context.fillEllipse(in: CGRect(x: center.x - dot, y: center.y - dot, width: dot * 2, height: dot * 2))
    return context.makeImage()
}

let arguments = CommandLine.arguments
guard arguments.count == 2 else {
    fail("usage: swift make-icon.swift <output.iconset>")
}
let output = URL(fileURLWithPath: arguments[1], isDirectory: true)
do {
    try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
} catch {
    fail("cannot create \(output.path): \(error)")
}

// Every size iconutil expects, at 1x and 2x.
for base in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let name = scale == 1 ? "icon_\(base)x\(base).png" : "icon_\(base)x\(base)@2x.png"
        let url = output.appendingPathComponent(name) as CFURL
        guard let image = drawIcon(pixels: base * scale) else {
            fail("cannot draw \(name)")
        }
        guard let destination = CGImageDestinationCreateWithURL(url, UTType.png.identifier as CFString, 1, nil) else {
            fail("cannot write \(name)")
        }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else {
            fail("cannot encode \(name)")
        }
    }
}
