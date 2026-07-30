//
//  SimulationCanvasView.swift
//  FingerFlow
//
//  Wraps an MTKView for SwiftUI and forwards single-finger strokes to the
//  simulation as paint commands.
//

import MetalKit
import SwiftUI
import UIKit

struct SimulationCanvasView: UIViewRepresentable {
    let sim: FluidSimulation

    func makeUIView(context: Context) -> TouchMTKView {
        let view = TouchMTKView(frame: .zero, device: sim.mtlDevice)
        view.sim = sim
        view.delegate = sim
        view.colorPixelFormat = .bgra8Unorm
        view.framebufferOnly = false // the renderer writes via compute
        view.preferredFramesPerSecond = 60
        view.isPaused = false
        view.enableSetNeedsDisplay = false
        view.isMultipleTouchEnabled = false
        return view
    }

    func updateUIView(_ uiView: TouchMTKView, context: Context) {}
}

final class TouchMTKView: MTKView {
    weak var sim: FluidSimulation?
    private var lastPoint: CGPoint?
    /// Fan strokes defer their first stamp until the drag direction is
    /// known, so the whole stroke blows the way the finger moved.
    private var pendingFanStart: CGPoint?

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard let touch = touches.first else { return }
        let p = touch.location(in: self)
        sim?.beginStroke()
        if sim?.tool == .fan {
            pendingFanStart = p
        } else {
            sim?.paint(from: p, to: p, in: bounds.size)
        }
        lastPoint = p
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard let touch = touches.first else { return }
        let p = touch.location(in: self)
        if let start = pendingFanStart {
            // Wait for a clear direction before stamping the fan stroke.
            guard hypot(p.x - start.x, p.y - start.y) >= 8 else { return }
            pendingFanStart = nil
            sim?.paint(from: start, to: p, in: bounds.size)
        } else {
            sim?.paint(from: lastPoint ?? p, to: p, in: bounds.size)
        }
        lastPoint = p
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        if let start = pendingFanStart {
            // A tap: stamp with the tunnel-aligned default direction.
            pendingFanStart = nil
            sim?.paint(from: start, to: start, in: bounds.size)
        }
        lastPoint = nil
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        pendingFanStart = nil
        lastPoint = nil
    }
}
