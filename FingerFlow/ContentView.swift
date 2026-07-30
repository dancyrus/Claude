//
//  ContentView.swift
//  FingerFlow
//
//  Full-screen simulation canvas with a floating control overlay.
//

import SwiftUI

struct ContentView: View {
    @StateObject private var sim = FluidSimulation()
    @State private var showSettings = false

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()
            SimulationCanvasView(sim: sim)
                .ignoresSafeArea()

            VStack(spacing: 0) {
                TopBarView(sim: sim, showSettings: $showSettings)
                Spacer()
                ToolbarView(sim: sim)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
        }
        .sheet(isPresented: $showSettings) {
            SettingsView(sim: sim)
        }
        .preferredColorScheme(.dark)
        .statusBarHidden()
        .persistentSystemOverlays(.hidden)
    }
}

#Preview {
    ContentView()
}
