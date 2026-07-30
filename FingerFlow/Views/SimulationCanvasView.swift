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

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard let touch = touches.first else { return }
        let p = touch.location(in: self)
        sim?.paint(from: p, to: p, in: bounds.size)
        lastPoint = p
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard let touch = touches.first else { return }
        let p = touch.location(in: self)
        sim?.paint(from: lastPoint ?? p, to: p, in: bounds.size)
        lastPoint = p
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        lastPoint = nil
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        lastPoint = nil
    }
}
