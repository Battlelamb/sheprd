import AVFoundation

/// The choices in Settings > Voice.
enum VoiceSettings {
    static let speaksKey = "voiceSpeaks"
    static let confirmsKey = "voiceConfirms"
    static let languageKey = "voicePhoneLanguage"

    static var speaks: Bool { UserDefaults.standard.object(forKey: speaksKey) as? Bool ?? true }
    /// A spoken yes starts a new worktree, for use without looking.
    static var confirms: Bool { UserDefaults.standard.bool(forKey: confirmsKey) }
    /// Commands in the phone's language rather than English. Only the
    /// model can read those, and less reliably.
    static var phoneLanguage: Bool { UserDefaults.standard.bool(forKey: languageKey) }
    static var english: Bool { !phoneLanguage || Locale.current.language.languageCode == .english }
}


@MainActor
final class VoiceSpeaker: NSObject, AVSpeechSynthesizerDelegate, AVAudioPlayerDelegate {
    private let synth = AVSpeechSynthesizer()
    private var utterance: AVSpeechUtterance?
    private var player: AVAudioPlayer?
    private var download: Task<Data, Error>?
    private var completion: CheckedContinuation<Bool, Never>?
    private var generation = UUID()
    private var ownsAudio = false

    override init() {
        super.init()
        synth.delegate = self
        NotificationCenter.default.addObserver(self, selector: #selector(interrupted(_:)),
            name: AVAudioSession.interruptionNotification, object: AVAudioSession.sharedInstance())
    }

    @objc private nonisolated func interrupted(_ note: Notification) {
        guard note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt == AVAudioSession.InterruptionType.began.rawValue else { return }
        Task { @MainActor in if self.ownsAudio { self.stop() } }
    }

    @discardableResult
    func say(_ words: String, always: Bool = false) async -> Bool {
        guard always || VoiceSettings.speaks else { return false }
        do { return try await play(words, account: SpeechAccount.load()) }
        catch { return false }
    }

    func play(_ words: String, account: SpeechAccount?) async throws -> Bool {
        stop()
        let generation = self.generation
        defer { if self.generation == generation { stop() } }
        if let account {
            let task = Task { try await account.audio(words) }
            download = task
            let data = try await task.value
            guard self.generation == generation else { return false }
            player = try AVAudioPlayer(data: data)
            player?.delegate = self
        }
        let session = AVAudioSession.sharedInstance()
        Feedback.shared.holdAudio(self)
        ownsAudio = true
        try session.setCategory(.playback, options: .duckOthers)
        try session.setActive(true)
        return await withCheckedContinuation { continuation in
            completion = continuation
            if let player {
                if !player.play() { finish(false) }
            } else {
                let utterance = AVSpeechUtterance(string: words)
                self.utterance = utterance
                synth.speak(utterance)
            }
        }
    }

    func stop() {
        generation = UUID()
        download?.cancel()
        download = nil
        utterance = nil
        player?.stop()
        player = nil
        synth.stopSpeaking(at: .immediate)
        finish(false)
        if ownsAudio {
            ownsAudio = false
            try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
            Feedback.shared.releaseAudio(self)
        }
    }

    private func finish(_ success: Bool) {
        completion?.resume(returning: success)
        completion = nil
    }

    nonisolated func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance) {
        let id = ObjectIdentifier(utterance)
        Task { @MainActor in if self.utterance.map(ObjectIdentifier.init) == id { self.finish(true) } }
    }

    nonisolated func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didCancel utterance: AVSpeechUtterance) {
        let id = ObjectIdentifier(utterance)
        Task { @MainActor in if self.utterance.map(ObjectIdentifier.init) == id { self.finish(false) } }
    }

    nonisolated func audioPlayerDidFinishPlaying(_ player: AVAudioPlayer, successfully flag: Bool) {
        let id = ObjectIdentifier(player)
        Task { @MainActor in if self.player.map(ObjectIdentifier.init) == id { self.finish(flag) } }
    }

    nonisolated func audioPlayerDecodeErrorDidOccur(_ player: AVAudioPlayer, error: Error?) {
        audioPlayerDidFinishPlaying(player, successfully: false)
    }
}
