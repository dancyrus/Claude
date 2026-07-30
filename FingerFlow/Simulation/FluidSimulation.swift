//
//  FluidSimulation.swift
//  FingerFlow
//
//  Owns the Metal state and the lattice-Boltzmann grid, drives the
//  simulation from the MTKView draw loop, and applies finger painting
//  (walls, fans, outlets, dye) directly into shared-memory buffers.
//

import Foundation
import Metal
import MetalKit
import SwiftUI
import UIKit
import simd

// MARK: - UI-facing enums

enum Tool: String, CaseIterable, Identifiable {
    case wall, fan, dye, outlet, eraser

    var id: String { rawValue }

    var label: String {
        switch self {
        case .wall:   return "Wall"
        case .fan:    return "Fan"
        case .dye:    return "Smoke"
        case .outlet: return "Drain"
        case .eraser: return "Erase"
        }
    }

    var icon: String {
        switch self {
        case .wall:   return "pencil"
        case .fan:    return "wind"
        case .dye:    return "smoke.fill"
        case .outlet: return "arrow.down.circle"
        case .eraser: return "eraser.fill"
        }
    }
}

enum RenderMode: Int32, CaseIterable, Identifiable {
    case dye = 0, speed, vorticity, pressure

    var id: Int32 { rawValue }

    var label: String {
        switch self {
        case .dye:       return "Smoke"
        case .speed:     return "Speed"
        case .vorticity: return "Vorticity"
        case .pressure:  return "Pressure"
        }
    }

    var icon: String {
        switch self {
        case .dye:       return "smoke"
        case .speed:     return "gauge.with.needle"
        case .vorticity: return "hurricane"
        case .pressure:  return "barometer"
        }
    }
}

enum Preset: String, CaseIterable, Identifiable {
    case cylinder = "Cylinder"
    case airfoil  = "Airfoil"
    case venturi  = "Venturi"
    case step     = "Step"
    case pinball  = "Pinball"

    var id: String { rawValue }
}

// MARK: - Simulation

final class FluidSimulation: NSObject, ObservableObject, MTKViewDelegate {

    // MARK: Published controls

    @Published var tool: Tool = .wall
    @Published var renderMode: RenderMode = .dye
    @Published var isPaused = false
    /// Lattice kinematic viscosity. tau = 3*nu + 0.5 must stay above 0.5.
    @Published var viscosity: Double = 0.015
    /// Lattice speed of the tunnel inflow and painted fans (keep < ~0.15).
    @Published var flowSpeed: Double = 0.09
    @Published var stepsPerFrame: Double = 6
    /// Brush radius in grid cells.
    @Published var brushRadius: Double = 6
    /// Per-frame dye retention (1 = never fades).
    @Published var dyeFade: Double = 0.995
    @Published var windTunnel = true {
        didSet { applyWindTunnel(windTunnel) }
    }
    @Published var dyeColor: Color = Color(red: 0.35, green: 0.85, blue: 1.0)

    // MARK: Metal state

    private struct MetalState {
        let device: MTLDevice
        let queue: MTLCommandQueue
        let collide: MTLComputePipelineState
        let advect: MTLComputePipelineState
        let render: MTLComputePipelineState
    }

    private struct Grid {
        var w: Int
        var h: Int
        var fA: MTLBuffer        // 9 * n floats (ping)
        var fB: MTLBuffer        // 9 * n floats (pong)
        var vel: MTLBuffer       // n float2
        var rho: MTLBuffer       // n float
        var cellType: MTLBuffer  // n uchar
        var fanDir: MTLBuffer    // n float2 (unit direction for inlet cells)
        var dyeA: MTLBuffer      // n float4
        var dyeB: MTLBuffer      // n float4
        var dyeSrc: MTLBuffer    // n float4 (rgb + strength)
        var n: Int { w * h }
    }

    private var metal: MetalState?
    private var grid: Grid?
    private var didLoadInitialScene = false
    private var lastFanDir = SIMD2<Float>(0, -1)
    private var strokeHasDirection = false
    /// Most recently committed frame; CPU-side buffer writes wait on it so
    /// they never race the GPU passes that read the same shared buffers.
    private var inFlight: MTLCommandBuffer?

    private static let weights: [Float] = [
        4.0 / 9.0,
        1.0 / 9.0, 1.0 / 9.0, 1.0 / 9.0, 1.0 / 9.0,
        1.0 / 36.0, 1.0 / 36.0, 1.0 / 36.0, 1.0 / 36.0,
    ]

    /// Cells across the short side of the screen.
    private static let targetResolution = 224.0

    var mtlDevice: MTLDevice? { metal?.device }

    override init() {
        super.init()
        guard let device = MTLCreateSystemDefaultDevice(),
              let queue = device.makeCommandQueue(),
              let library = device.makeDefaultLibrary(),
              let fCollide = library.makeFunction(name: "collideStream"),
              let fAdvect = library.makeFunction(name: "advectDye"),
              let fRender = library.makeFunction(name: "renderField"),
              let pCollide = try? device.makeComputePipelineState(function: fCollide),
              let pAdvect = try? device.makeComputePipelineState(function: fAdvect),
              let pRender = try? device.makeComputePipelineState(function: fRender)
        else { return }
        metal = MetalState(device: device, queue: queue,
                           collide: pCollide, advect: pAdvect, render: pRender)
    }

    // MARK: - Grid lifecycle

    /// Blocks until the last committed frame finishes so CPU writes to the
    /// shared buffers cannot interleave with in-flight GPU passes. Frames
    /// take ~1-2 ms, so this is imperceptible on user actions.
    private func waitForGPU() {
        inFlight?.waitUntilCompleted()
        inFlight = nil
    }

    private func rebuildGridIfNeeded(drawableSize: CGSize) {
        guard let ms = metal, drawableSize.width > 0, drawableSize.height > 0 else { return }
        let scale = Self.targetResolution / Double(min(drawableSize.width, drawableSize.height))
        var w = Int((Double(drawableSize.width) * scale).rounded())
        var h = Int((Double(drawableSize.height) * scale).rounded())
        w = max(64, min(w, 640))
        h = max(64, min(h, 640))
        if let g = grid, abs(g.w - w) <= 4, abs(g.h - h) <= 4 { return }

        waitForGPU()

        // Strip tunnel edge cells from the old grid so they aren't
        // resampled into the interior of the new one.
        if grid != nil, windTunnel { applyWindTunnel(false) }
        let old = grid

        let n = w * h
        guard
            let fA = ms.device.makeBuffer(length: 9 * n * MemoryLayout<Float>.stride, options: .storageModeShared),
            let fB = ms.device.makeBuffer(length: 9 * n * MemoryLayout<Float>.stride, options: .storageModeShared),
            let vel = ms.device.makeBuffer(length: n * MemoryLayout<SIMD2<Float>>.stride, options: .storageModeShared),
            let rho = ms.device.makeBuffer(length: n * MemoryLayout<Float>.stride, options: .storageModeShared),
            let cellType = ms.device.makeBuffer(length: n * MemoryLayout<UInt8>.stride, options: .storageModeShared),
            let fanDir = ms.device.makeBuffer(length: n * MemoryLayout<SIMD2<Float>>.stride, options: .storageModeShared),
            let dyeA = ms.device.makeBuffer(length: n * MemoryLayout<SIMD4<Float>>.stride, options: .storageModeShared),
            let dyeB = ms.device.makeBuffer(length: n * MemoryLayout<SIMD4<Float>>.stride, options: .storageModeShared),
            let dyeSrc = ms.device.makeBuffer(length: n * MemoryLayout<SIMD4<Float>>.stride, options: .storageModeShared)
        else { return }

        grid = Grid(w: w, h: h, fA: fA, fB: fB, vel: vel, rho: rho,
                    cellType: cellType, fanDir: fanDir,
                    dyeA: dyeA, dyeB: dyeB, dyeSrc: dyeSrc)
        clearGeometry()
        if let old, let new = grid {
            resample(from: old, into: new)
        }
        resetFlow()
        if windTunnel { applyWindTunnel(true) }
        if !didLoadInitialScene {
            didLoadInitialScene = true
            apply(preset: .cylinder)
        }
    }

    /// Nearest-neighbour copy of the drawn scene (walls, fans, drains, dye
    /// sources) into a freshly sized grid, so rotation doesn't wipe it.
    private func resample(from old: Grid, into new: Grid) {
        let oct = old.cellType.contents().bindMemory(to: UInt8.self, capacity: old.n)
        let ofan = old.fanDir.contents().bindMemory(to: SIMD2<Float>.self, capacity: old.n)
        let osrc = old.dyeSrc.contents().bindMemory(to: SIMD4<Float>.self, capacity: old.n)
        let nct = cellTypePtr(new)
        let nfan = fanDirPtr(new)
        let nsrc = dyeSrcPtr(new)
        for y in 0..<new.h {
            let oy = min(old.h - 1, y * old.h / new.h)
            for x in 0..<new.w {
                let ox = min(old.w - 1, x * old.w / new.w)
                let oi = oy * old.w + ox
                let ni = y * new.w + x
                nct[ni] = oct[oi]
                nfan[ni] = ofan[oi]
                nsrc[ni] = osrc[oi]
            }
        }
    }

    // MARK: Typed buffer views

    private func cellTypePtr(_ g: Grid) -> UnsafeMutablePointer<UInt8> {
        g.cellType.contents().bindMemory(to: UInt8.self, capacity: g.n)
    }
    private func fanDirPtr(_ g: Grid) -> UnsafeMutablePointer<SIMD2<Float>> {
        g.fanDir.contents().bindMemory(to: SIMD2<Float>.self, capacity: g.n)
    }
    private func dyeSrcPtr(_ g: Grid) -> UnsafeMutablePointer<SIMD4<Float>> {
        g.dyeSrc.contents().bindMemory(to: SIMD4<Float>.self, capacity: g.n)
    }

    // MARK: - Scene mutation (all on the main thread)

    /// Reinitialise the flow to rest without touching the drawn geometry.
    func resetFlow() {
        guard let g = grid else { return }
        waitForGPU()
        let n = g.n
        let fA = g.fA.contents().bindMemory(to: Float.self, capacity: 9 * n)
        let fB = g.fB.contents().bindMemory(to: Float.self, capacity: 9 * n)
        for i in 0..<9 {
            let w = Self.weights[i]
            for c in 0..<n {
                fA[i * n + c] = w
                fB[i * n + c] = w
            }
        }
        let vel = g.vel.contents().bindMemory(to: SIMD2<Float>.self, capacity: n)
        let rho = g.rho.contents().bindMemory(to: Float.self, capacity: n)
        let dyeA = g.dyeA.contents().bindMemory(to: SIMD4<Float>.self, capacity: n)
        let dyeB = g.dyeB.contents().bindMemory(to: SIMD4<Float>.self, capacity: n)
        for c in 0..<n {
            vel[c] = .zero
            rho[c] = 1
            dyeA[c] = .zero
            dyeB[c] = .zero
        }
    }

    /// Remove all painted geometry, fans, outlets and dye sources.
    private func clearGeometry() {
        guard let g = grid else { return }
        let ct = cellTypePtr(g)
        let fan = fanDirPtr(g)
        let src = dyeSrcPtr(g)
        for c in 0..<g.n {
            ct[c] = UInt8(CELL_FLUID)
            fan[c] = .zero
            src[c] = .zero
        }
    }

    /// Clear everything and start from a still, empty domain.
    func clearAll() {
        waitForGPU()
        clearGeometry()
        resetFlow()
        if windTunnel { applyWindTunnel(true) }
    }

    /// The wind tunnel runs along the screen's long axis: bottom-to-top in
    /// portrait, left-to-right in landscape. Two rows of inlet cells feed
    /// the domain, the far edge drains it, and a few dye streaklines are
    /// seeded at the inlet.
    private func applyWindTunnel(_ enable: Bool) {
        guard let g = grid else { return }
        waitForGPU()
        let ct = cellTypePtr(g)
        let fan = fanDirPtr(g)
        let src = dyeSrcPtr(g)
        let vertical = g.h >= g.w
        let streak = SIMD4<Float>(0.92, 0.94, 1.0, 0.9)

        func setCell(_ x: Int, _ y: Int, type: Int32, dir: SIMD2<Float>, dye: SIMD4<Float>) {
            let i = y * g.w + x
            ct[i] = UInt8(type)
            fan[i] = dir
            src[i] = dye
        }

        if vertical {
            for x in 0..<g.w {
                let seed = (x % 12) < 2
                for r in 0..<2 {
                    let dye = (enable && seed) ? streak : SIMD4<Float>.zero
                    // Inlet at the bottom edge blowing up the screen.
                    setCell(x, g.h - 1 - r,
                            type: enable ? CELL_INLET : CELL_FLUID,
                            dir: enable ? SIMD2<Float>(0, -1) : .zero,
                            dye: dye)
                    // Outlet at the top edge.
                    setCell(x, r,
                            type: enable ? CELL_OUTLET : CELL_FLUID,
                            dir: .zero, dye: .zero)
                }
            }
        } else {
            for y in 0..<g.h {
                let seed = (y % 12) < 2
                for r in 0..<2 {
                    let dye = (enable && seed) ? streak : SIMD4<Float>.zero
                    // Inlet at the left edge blowing right.
                    setCell(r, y,
                            type: enable ? CELL_INLET : CELL_FLUID,
                            dir: enable ? SIMD2<Float>(1, 0) : .zero,
                            dye: dye)
                    // Outlet at the right edge.
                    setCell(g.w - 1 - r, y,
                            type: enable ? CELL_OUTLET : CELL_FLUID,
                            dir: .zero, dye: .zero)
                }
            }
        }
    }

    // MARK: - Presets

    func apply(preset: Preset) {
        guard let g = grid else { return }
        waitForGPU()
        clearGeometry()
        resetFlow()
        if !windTunnel {
            windTunnel = true // didSet re-applies the tunnel cells
        } else {
            applyWindTunnel(true)
        }

        let ct = cellTypePtr(g)
        let vertical = g.h >= g.w
        let along = Float(vertical ? g.h : g.w) // length along the flow
        let cross = Float(vertical ? g.w : g.h) // width across the flow

        // Stamp walls wherever the predicate over flow-aligned coordinates
        // holds: s in cells along the flow, t in cells across it.
        func stampWalls(_ isWall: (Float, Float) -> Bool) {
            for y in 0..<g.h {
                for x in 0..<g.w {
                    let s = vertical ? Float(g.h - 1 - y) : Float(x)
                    let t = vertical ? Float(x) : Float(y)
                    if isWall(s, t) {
                        let i = y * g.w + x
                        if ct[i] == UInt8(CELL_FLUID) { ct[i] = UInt8(CELL_WALL) }
                    }
                }
            }
        }

        func circle(_ cs: Float, _ ctr: Float, _ r: Float, _ s: Float, _ t: Float) -> Bool {
            let ds = s - cs * along
            let dt = t - ctr * cross
            return ds * ds + dt * dt <= r * r
        }

        switch preset {
        case .cylinder:
            let r = 0.08 * cross
            stampWalls { s, t in circle(0.30, 0.5, r, s, t) }

        case .airfoil:
            // NACA 0012 at ~10 degrees angle of attack.
            let chord = 0.5 * along
            let alpha: Float = 10 * .pi / 180
            let ls = 0.22 * along // leading edge along the flow
            let lt = 0.48 * cross
            stampWalls { s, t in
                let ds = s - ls
                let dt = t - lt
                let xc = ds * cos(alpha) + dt * sin(alpha)
                let yc = -ds * sin(alpha) + dt * cos(alpha)
                guard xc >= 0, xc <= chord else { return false }
                let xn = xc / chord
                let yt = 0.6 * chord * (0.2969 * sqrt(xn) - 0.1260 * xn
                    - 0.3516 * xn * xn + 0.2843 * xn * xn * xn
                    - 0.1015 * xn * xn * xn * xn)
                return abs(yc) <= yt
            }

        case .venturi:
            // Smooth constriction of the channel around mid-domain.
            stampWalls { s, t in
                let x = (s / along - 0.45) / 0.16
                let gap = 1.0 - 0.62 * exp(-x * x)
                let half = 0.5 * gap * cross
                let mid = 0.5 * cross
                return abs(t - mid) > half
            }

        case .step:
            // Backward-facing step.
            stampWalls { s, t in s < 0.32 * along && t < 0.5 * cross }

        case .pinball:
            let r = 0.055 * cross
            let centers: [(Float, Float)] = [
                (0.28, 0.30), (0.28, 0.70),
                (0.48, 0.50),
                (0.68, 0.30), (0.68, 0.70),
            ]
            stampWalls { s, t in
                centers.contains { circle($0.0, $0.1, r, s, t) }
            }
        }
    }

    // MARK: - Painting

    /// Marks the start of a new finger stroke (resets the per-stroke fan
    /// direction so a tap doesn't inherit an old stroke's direction).
    func beginStroke() {
        strokeHasDirection = false
    }

    /// Fan direction used for taps: along the wind tunnel's flow axis.
    private var defaultFanDir: SIMD2<Float> {
        guard let g = grid else { return SIMD2<Float>(0, -1) }
        return g.h >= g.w ? SIMD2<Float>(0, -1) : SIMD2<Float>(1, 0)
    }

    /// Paint a stroke segment. Points are in the MTKView's coordinate space
    /// (points, origin top-left); `viewSize` is the view's bounds size.
    func paint(from a: CGPoint, to b: CGPoint, in viewSize: CGSize) {
        guard let g = grid, viewSize.width > 0, viewSize.height > 0 else { return }
        waitForGPU()
        let sx = CGFloat(g.w) / viewSize.width
        let sy = CGFloat(g.h) / viewSize.height
        let p0 = SIMD2<Float>(Float(a.x * sx), Float(a.y * sy))
        let p1 = SIMD2<Float>(Float(b.x * sx), Float(b.y * sy))
        let delta = p1 - p0
        let len = simd_length(delta)

        // Fans blow along the stroke direction; a plain tap blows along
        // the tunnel axis instead of inheriting an old stroke's direction.
        if tool == .fan {
            if len > 1.0 {
                lastFanDir = delta / len
                strokeHasDirection = true
            } else if !strokeHasDirection {
                lastFanDir = defaultFanDir
            }
        }

        let r = Float(max(1.5, brushRadius))
        let stepCount = max(1, Int(len / max(r * 0.4, 1)))
        for s in 0...stepCount {
            let t = Float(s) / Float(stepCount)
            stamp(at: p0 + delta * t, radius: r, grid: g)
        }
    }

    private func stamp(at c: SIMD2<Float>, radius r: Float, grid g: Grid) {
        let ct = cellTypePtr(g)
        let fan = fanDirPtr(g)
        let src = dyeSrcPtr(g)
        let rgb = dyeRGB

        let x0 = max(0, Int(c.x - r)), x1 = min(g.w - 1, Int(c.x + r))
        let y0 = max(0, Int(c.y - r)), y1 = min(g.h - 1, Int(c.y + r))
        guard x0 <= x1, y0 <= y1 else { return }

        for y in y0...y1 {
            for x in x0...x1 {
                let dx = Float(x) + 0.5 - c.x
                let dy = Float(y) + 0.5 - c.y
                guard dx * dx + dy * dy <= r * r else { continue }
                let i = y * g.w + x
                switch tool {
                case .wall:
                    ct[i] = UInt8(CELL_WALL)
                    fan[i] = .zero
                    src[i] = .zero
                case .fan:
                    ct[i] = UInt8(CELL_INLET)
                    fan[i] = lastFanDir
                    src[i] = SIMD4<Float>(rgb.x, rgb.y, rgb.z, 0.8)
                case .outlet:
                    ct[i] = UInt8(CELL_OUTLET)
                    fan[i] = .zero
                    src[i] = .zero
                case .dye:
                    if ct[i] == UInt8(CELL_FLUID) {
                        src[i] = SIMD4<Float>(rgb.x, rgb.y, rgb.z, 1.0)
                    }
                case .eraser:
                    // Only reinitialise cells that were solid or boundary;
                    // erasing over open fluid must not punch a still hole
                    // into the flow.
                    let wasFluid = ct[i] == UInt8(CELL_FLUID)
                    ct[i] = UInt8(CELL_FLUID)
                    fan[i] = .zero
                    src[i] = .zero
                    if !wasFluid { resetCell(i, grid: g) }
                }
            }
        }
    }

    /// Reinitialise one cell's populations to rest equilibrium (used when
    /// erasing a wall so the uncovered cell holds sane values).
    private func resetCell(_ i: Int, grid g: Grid) {
        let n = g.n
        let fA = g.fA.contents().bindMemory(to: Float.self, capacity: 9 * n)
        let fB = g.fB.contents().bindMemory(to: Float.self, capacity: 9 * n)
        for k in 0..<9 {
            fA[k * n + i] = Self.weights[k]
            fB[k * n + i] = Self.weights[k]
        }
        // Keep the derived fields consistent even while paused (they are
        // otherwise only rewritten by the collide kernel).
        g.vel.contents().bindMemory(to: SIMD2<Float>.self, capacity: n)[i] = .zero
        g.rho.contents().bindMemory(to: Float.self, capacity: n)[i] = 1
    }

    private var dyeRGB: SIMD3<Float> {
        let ui = UIColor(dyeColor)
        var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
        if ui.getRed(&r, green: &g, blue: &b, alpha: &a) {
            return SIMD3<Float>(Float(r), Float(g), Float(b))
        }
        return SIMD3<Float>(0.35, 0.85, 1.0)
    }

    /// Rough Reynolds number for the current settings, using a
    /// cylinder-preset-sized obstacle as the characteristic length.
    var reynoldsEstimate: Int {
        guard let g = grid else { return 0 }
        let cross = Double(min(g.w, g.h))
        let L = 0.16 * cross
        return Int((flowSpeed * L / max(viscosity, 1e-5)).rounded())
    }

    var gridDescription: String {
        guard let g = grid else { return "—" }
        return "\(g.w) × \(g.h)"
    }

    // MARK: - MTKViewDelegate

    func mtkView(_ view: MTKView, drawableSizeWillChange size: CGSize) {
        rebuildGridIfNeeded(drawableSize: size)
    }

    func draw(in view: MTKView) {
        if grid == nil {
            rebuildGridIfNeeded(drawableSize: view.drawableSize)
        }
        guard let ms = metal, var g = grid,
              let drawable = view.currentDrawable,
              let cmd = ms.queue.makeCommandBuffer()
        else { return }

        let steps = isPaused ? 0 : max(1, Int(stepsPerFrame.rounded()))
        var params = SimParams(
            width: Int32(g.w),
            height: Int32(g.h),
            omega: Float(1.0 / (3.0 * max(viscosity, 0.004) + 0.5)),
            inletSpeed: Float(flowSpeed),
            dyeDt: Float(steps),
            dyeDecay: isPaused ? 1.0 : Float(dyeFade),
            renderMode: renderMode.rawValue
        )

        // LBM steps (ping-pong the distribution buffers).
        for _ in 0..<steps {
            guard let enc = cmd.makeComputeCommandEncoder() else { break }
            enc.setComputePipelineState(ms.collide)
            enc.setBuffer(g.fA, offset: 0, index: 0)
            enc.setBuffer(g.fB, offset: 0, index: 1)
            enc.setBuffer(g.cellType, offset: 0, index: 2)
            enc.setBuffer(g.fanDir, offset: 0, index: 3)
            enc.setBuffer(g.vel, offset: 0, index: 4)
            enc.setBuffer(g.rho, offset: 0, index: 5)
            enc.setBytes(&params, length: MemoryLayout<SimParams>.stride, index: 6)
            dispatch(enc, ms.collide, over: g.w, g.h)
            enc.endEncoding()
            swap(&g.fA, &g.fB)
        }

        // Dye advection once per frame (also runs while paused so painted
        // smoke sources show up immediately; dyeDt is 0 then).
        if let enc = cmd.makeComputeCommandEncoder() {
            enc.setComputePipelineState(ms.advect)
            enc.setBuffer(g.dyeA, offset: 0, index: 0)
            enc.setBuffer(g.dyeB, offset: 0, index: 1)
            enc.setBuffer(g.vel, offset: 0, index: 2)
            enc.setBuffer(g.cellType, offset: 0, index: 3)
            enc.setBuffer(g.dyeSrc, offset: 0, index: 4)
            enc.setBytes(&params, length: MemoryLayout<SimParams>.stride, index: 5)
            dispatch(enc, ms.advect, over: g.w, g.h)
            enc.endEncoding()
            swap(&g.dyeA, &g.dyeB)
        }

        // Visualise straight into the drawable.
        if let enc = cmd.makeComputeCommandEncoder() {
            let tex = drawable.texture
            enc.setComputePipelineState(ms.render)
            enc.setTexture(tex, index: 0)
            enc.setBuffer(g.vel, offset: 0, index: 0)
            enc.setBuffer(g.rho, offset: 0, index: 1)
            enc.setBuffer(g.dyeA, offset: 0, index: 2)
            enc.setBuffer(g.cellType, offset: 0, index: 3)
            enc.setBytes(&params, length: MemoryLayout<SimParams>.stride, index: 4)
            dispatch(enc, ms.render, over: tex.width, tex.height)
            enc.endEncoding()
        }

        cmd.present(drawable)
        cmd.commit()
        inFlight = cmd
        grid = g // persist the ping-pong swaps
    }

    private func dispatch(_ enc: MTLComputeCommandEncoder,
                          _ pipeline: MTLComputePipelineState,
                          over w: Int, _ h: Int) {
        let tw = max(1, pipeline.threadExecutionWidth)
        let th = max(1, min(16, pipeline.maxTotalThreadsPerThreadgroup / tw))
        let tpg = MTLSize(width: tw, height: th, depth: 1)
        let groups = MTLSize(width: (w + tw - 1) / tw,
                             height: (h + th - 1) / th,
                             depth: 1)
        enc.dispatchThreadgroups(groups, threadsPerThreadgroup: tpg)
    }
}
