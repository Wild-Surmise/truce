import AudioToolbox
import AVFAudio
import Foundation

let mode = CommandLine.arguments[1]
if mode == "dirty-previous-abi" { mock_use_previous_abi() }

let au = try TruceAUAudioUnit(componentDescription: AudioComponentDescription(
    componentType: kAudioUnitType_Effect, componentSubType: 0x74737431,
    componentManufacturer: 0x54737431, componentFlags: 0, componentFlagsMask: 0))
if mode == "dirty" || mode == "dirty-previous-abi" {
    var documentCount = 0
    var presetCount = 0
    let documentObservation = au.observe(\.fullStateForDocument, options: [.new]) { _, change in
        precondition(change.newValue??["truce_state"] as? Data != nil,
                     "document state was unavailable during notification")
        documentCount += 1
    }
    let presetObservation = au.observe(\.fullState, options: [.new]) { _, change in
        precondition(change.newValue??["truce_state"] as? Data != nil,
                     "preset state was unavailable during notification")
        presetCount += 1
    }
    au.notifyDocumentStateIfDirty()
    precondition(documentCount == 0 && presetCount == 0, "notified without an edit")
    mock_mark_state_dirty()
    mock_mark_state_dirty()
    au.notifyDocumentStateIfDirty()
    au.notifyDocumentStateIfDirty()
    let expected = mode == "dirty" ? 1 : 0
    precondition(documentCount == expected && presetCount == expected,
                 "expected \(expected) notifications per property; document \(documentCount), preset \(presetCount)")
    documentObservation.invalidate()
    presetObservation.invalidate()
    print("PASS \(mode)")
    exit(0)
}

let gain = au.parameterTree!.allParameters[0]
gain.value = 0.75
var state = au.fullState!
let blob = state["truce_state"] as! Data
if mode != "roundtrip" && mode != "apple-only" {
    // Invalid archive tag reproduces AUParameterGroup._deserialize + 176.
    var apple = state["data"] as! Data
    apple[0] = 2
    state["data"] = apple
}
if mode == "legacy" {
    state.removeValue(forKey: "truce_state")
    state["legacy_state"] = blob
}
if mode == "apple-only" || mode.hasSuffix("apple-only") || mode == "partial" {
    state.removeValue(forKey: "truce_state")
}
if mode == "invalid-type" || mode == "invalid-type-apple-only" {
    state["data"] = "not an Apple archive"
}
if mode == "empty-apple-only" { state["data"] = Data() }
if mode == "partial" {
    // Apply the first parameter, then throw on the terminator. The failed
    // fallback must roll back any parameter writes already made by Apple.
    var apple = au.fullState!["data"] as! Data
    apple[apple.count - 1] = 2
    state["data"] = apple
}
if mode == "rejected-legacy" {
    state.removeValue(forKey: "truce_state")
    state["legacy_state"] = Data([0])
    state["data"] = au.fullState!["data"]
}
gain.value = 0.25
au.fullStateForDocument = mode == "nil" ? nil : state
let unchanged = ["invalid-apple-only", "invalid-type-apple-only", "empty-apple-only", "partial", "nil"]
let expected: Float = unchanged.contains(mode) ? 0.25 : 0.75
precondition(abs(gain.value - expected) < 0.0001, "state did not restore: \(gain.value)")
print("PASS \(mode)")
