// "Models on this computer" (the Tauri app's LocalServers): Ollama and LM Studio as detected on
// this computer's loopback address. A running one is added with one click; nothing is ever
// downloaded. Detected again each time the tab opens, and with Look Again.

import SwiftUI
import PageLampKit
import PageLampModel

struct AiLocalServersSection: View {
    let ai: AiSettingsModel
    /// A server was added: its disclosure comes next.
    let added: (ModelProviderRecord) -> Void

    @Environment(\.l10n) private var l10n

    var body: some View {
        Section {
            if let servers = ai.localServers {
                ForEach(servers, id: \.kind) { server in
                    row(server)
                }
            } else if ai.detecting {
                HStack(spacing: PLSpace.s2) {
                    ProgressView()
                        .controlSize(.small)
                        .accessibilityHidden(true)
                    Text(l10n("ai.local.detecting"))
                        .foregroundStyle(.secondary)
                }
            }
            if let failure = ai.localFailure {
                Text(l10n.sentences(l10n("ai.local.detectFailed"), l10n.aiError(failure)))
                    .foregroundStyle(PLColor.danger)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                Spacer()
                Button(l10n("mac.ai.lookAgain")) {
                    Task { await ai.detectLocalServers() }
                }
                .disabled(ai.detecting)
            }
        } header: {
            Text(l10n("ai.local.title"))
        } footer: {
            Text(l10n("ai.local.hint"))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private func row(_ server: LocalServer) -> some View {
        let name = l10n("ai.local.server.\(AiCodes.name(server.kind))")
        return HStack(alignment: .firstTextBaseline, spacing: PLSpace.s4) {
            VStack(alignment: .leading, spacing: 2) {
                Text(name)
                Text(server.running ? l10n("ai.local.running", ["address": server.baseUrl]) : l10n("ai.local.notRunning"))
                    .font(PLType.callout.font)
                    .foregroundStyle(.secondary)
                if let failure = ai.serverFailures[server.kind] {
                    Text(l10n.aiError(failure))
                        .font(PLType.callout.font)
                        .foregroundStyle(PLColor.danger)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if ai.isAdded(server) {
                Text(l10n("ai.local.inUse"))
                    .foregroundStyle(.secondary)
            } else if server.running {
                HStack(spacing: PLSpace.s2) {
                    if ai.addingServer == server.kind {
                        ProgressView()
                            .controlSize(.small)
                            .accessibilityHidden(true)
                    }
                    Button(l10n("ai.local.use", ["name": name])) {
                        Task {
                            if let record = await ai.useLocalServer(server) {
                                added(record)
                            } else if let failure = ai.serverFailures[server.kind] {
                                AccessibilityNotification.Announcement(l10n.aiError(failure)).post()
                            }
                        }
                    }
                    .disabled(ai.addingServer != nil)
                }
            }
        }
        .accessibilityElement(children: .contain)
    }
}
