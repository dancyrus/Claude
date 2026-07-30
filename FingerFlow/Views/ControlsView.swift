//
//  ControlsView.swift
//  FingerFlow
//
//  The floating game-style controls: playback + view controls on top,
//  drawing tools and brush size on the bottom, and a settings sheet.
//

import SwiftUI

// MARK: - Top bar

struct TopBarView: View {
    @ObservedObject var sim: FluidSimulation
    @Binding var showSettings: Bool

    var body: some View {
        HStack(spacing: 10) {
            Button {
                sim.isPaused.toggle()
            } label: {
                Image(systemName: sim.isPaused ? "play.fill" : "pause.fill")
                    .frame(width: 34, height: 34)
            }
            .buttonStyle(GlassButtonStyle())

            Button {
                sim.resetFlow()
            } label: {
                Image(systemName: "arrow.counterclockwise")
                    .frame(width: 34, height: 34)
            }
            .buttonStyle(GlassButtonStyle())

            Menu {
                ForEach(Preset.allCases) { preset in
                    Button(preset.rawValue) { sim.apply(preset: preset) }
                }
                Divider()
                Button(role: .destructive) {
                    sim.clearAll()
                } label: {
                    Label("Clear Everything", systemImage: "trash")
                }
            } label: {
                Image(systemName: "square.on.circle")
                    .frame(width: 34, height: 34)
            }
            .buttonStyle(GlassButtonStyle())

            Spacer()

            Menu {
                Picker("View", selection: $sim.renderMode) {
                    ForEach(RenderMode.allCases) { mode in
                        Label(mode.label, systemImage: mode.icon).tag(mode)
                    }
                }
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: sim.renderMode.icon)
                    Text(sim.renderMode.label)
                        .font(.footnote.weight(.semibold))
                }
                .frame(height: 34)
                .padding(.horizontal, 10)
            }
            .buttonStyle(GlassButtonStyle())

            Button {
                showSettings = true
            } label: {
                Image(systemName: "slider.horizontal.3")
                    .frame(width: 34, height: 34)
            }
            .buttonStyle(GlassButtonStyle())
        }
    }
}

// MARK: - Bottom toolbar

struct ToolbarView: View {
    @ObservedObject var sim: FluidSimulation

    var body: some View {
        VStack(spacing: 10) {
            HStack(spacing: 10) {
                Image(systemName: "circle.fill")
                    .font(.system(size: 7))
                    .foregroundStyle(.secondary)
                Slider(value: $sim.brushRadius, in: 2...20)
                Image(systemName: "circle.fill")
                    .font(.system(size: 16))
                    .foregroundStyle(.secondary)

                if sim.tool == .dye || sim.tool == .fan {
                    ColorPicker("", selection: $sim.dyeColor, supportsOpacity: false)
                        .labelsHidden()
                        .frame(width: 34)
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 6)
            .background(.ultraThinMaterial, in: Capsule())

            HStack(spacing: 6) {
                ForEach(Tool.allCases) { tool in
                    Button {
                        sim.tool = tool
                    } label: {
                        VStack(spacing: 3) {
                            Image(systemName: tool.icon)
                                .font(.system(size: 17, weight: .semibold))
                            Text(tool.label)
                                .font(.system(size: 10, weight: .medium))
                        }
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 8)
                        .foregroundStyle(sim.tool == tool ? Color.black : Color.primary)
                        .background(
                            sim.tool == tool ? AnyShapeStyle(Color.cyan) : AnyShapeStyle(Color.clear),
                            in: RoundedRectangle(cornerRadius: 12, style: .continuous)
                        )
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(6)
            .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        }
    }
}

// MARK: - Settings sheet

struct SettingsView: View {
    @ObservedObject var sim: FluidSimulation
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Form {
                Section("Fluid") {
                    LabeledSlider(title: "Flow speed",
                                  value: $sim.flowSpeed, in: 0.02...0.14,
                                  format: "%.3f")
                    LabeledSlider(title: "Viscosity",
                                  value: $sim.viscosity, in: 0.005...0.08,
                                  format: "%.3f")
                    LabeledSlider(title: "Sim speed (steps/frame)",
                                  value: $sim.stepsPerFrame, in: 1...10,
                                  format: "%.0f")
                    LabeledSlider(title: "Smoke persistence",
                                  value: $sim.dyeFade, in: 0.985...1.0,
                                  format: "%.3f")
                }

                Section("Domain") {
                    Toggle("Wind tunnel", isOn: $sim.windTunnel)
                    LabeledContent("Reynolds number (approx.)",
                                   value: "\(sim.reynoldsEstimate)")
                    LabeledContent("Grid", value: sim.gridDescription)
                }

                Section("Actions") {
                    Button("Reset flow (keep drawing)") { sim.resetFlow() }
                    Button("Clear everything", role: .destructive) { sim.clearAll() }
                }

                Section("About") {
                    Text("FingerFlow solves the 2D Navier–Stokes equations in "
                         + "real time with a D2Q9 lattice-Boltzmann method on "
                         + "the GPU. Draw walls, place fans and drains, blow "
                         + "smoke through your design — and hunt for vortex "
                         + "streets. Higher Reynolds numbers mean livelier, "
                         + "more turbulent flow.")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                }
            }
            .navigationTitle("Simulation")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
        .presentationDetents([.medium, .large])
    }
}

private struct LabeledSlider: View {
    let title: String
    @Binding var value: Double
    let range: ClosedRange<Double>
    let format: String

    init(title: String, value: Binding<Double>,
         in range: ClosedRange<Double>, format: String) {
        self.title = title
        self._value = value
        self.range = range
        self.format = format
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(title)
                Spacer()
                Text(String(format: format, value))
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }
            .font(.subheadline)
            Slider(value: $value, in: range)
        }
    }
}

// MARK: - Shared button chrome

struct GlassButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 15, weight: .semibold))
            .foregroundStyle(.primary)
            .background(.ultraThinMaterial, in: Capsule())
            .opacity(configuration.isPressed ? 0.6 : 1.0)
    }
}
