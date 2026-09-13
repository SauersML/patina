// Streams a summary line every 3 s of what a MIDI source sent, and writes
// exact note on/off/cc/bend events with timestamps to a raw log.
//   midi-listen <name-substring|all> <raw-log-path>
// CoreMIDI only delivers input to a process that pumps a run loop, hence the Timer.
import CoreMIDI
import Foundation
let filter = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "Hammer"
let rawPath = CommandLine.arguments.count > 2 ? CommandLine.arguments[2] : "/dev/null"
let names = ["C","C#","D","D#","E","F","F#","G","G#","A","A#","B"]
func nm(_ n: UInt8) -> String { "\(names[Int(n % 12)])\(Int(n / 12) - 1)" }
let lock = NSLock()
var notes: [(UInt8, UInt8, Double)] = []
var held = Set<UInt8>()
var ccs: [UInt8: UInt8] = [:]
var bend = false
let t0 = Date()
FileManager.default.createFile(atPath: rawPath, contents: nil)
let raw = FileHandle(forWritingAtPath: rawPath)!
func logRaw(_ s: String) { raw.write((s + "\n").data(using: .utf8)!); raw.synchronizeFile() }
var client = MIDIClientRef()
MIDIClientCreateWithBlock("listen" as CFString, &client) { _ in }
var port = MIDIPortRef()
MIDIInputPortCreateWithBlock(client, "in" as CFString, &port) { pktList, _ in
    var running: UInt8 = 0
    for packet in pktList.unsafeSequence() {
        let len = Int(packet.pointee.length)
        let bytes: [UInt8] = withUnsafeBytes(of: packet.pointee.data) { Array($0.prefix(len)) }
        var i = 0
        lock.lock()
        while i < bytes.count {
            var st = bytes[i]
            if st & 0x80 != 0 { running = st; i += 1 } else { st = running }
            if st >= 0xF0 { break }
            let kind = st & 0xF0
            let need = (kind == 0xC0 || kind == 0xD0) ? 1 : 2
            guard i + need <= bytes.count else { break }
            let d1 = bytes[i], d2 = need == 2 ? bytes[i + 1] : 0
            i += need
            let now = Date().timeIntervalSince(t0)
            switch kind {
            case 0x90 where d2 > 0: notes.append((d1, d2, now)); held.insert(d1); logRaw(String(format: "%.4f on %d %d", now, d1, d2))
            case 0x80, 0x90: held.remove(d1); logRaw(String(format: "%.4f off %d", now, d1))
            case 0xB0: ccs[d1] = d2; logRaw(String(format: "%.4f cc %d %d", now, d1, d2))
            case 0xE0: bend = true; logRaw(String(format: "%.4f bend %d", now, Int(d2) * 128 + Int(d1) - 8192))
            default: break
            }
        }
        lock.unlock()
    }
}
var connected = Set<MIDIEndpointRef>()
func connectAll() {
    for i in 0..<MIDIGetNumberOfSources() {
        let s = MIDIGetSource(i)
        if connected.contains(s) { continue }
        var cf: Unmanaged<CFString>?
        MIDIObjectGetStringProperty(s, kMIDIPropertyDisplayName, &cf)
        let name = (cf?.takeRetainedValue() as String?) ?? ""
        if filter == "all" || name.contains(filter) {
            MIDIPortConnectSource(port, s, nil); connected.insert(s)
            print("listening to \(name)"); fflush(stdout)
        }
    }
}
connectAll()
var quiet = 0
func summarize() {
    lock.lock()
    let n = notes; notes = []; let h = held; let c = ccs; ccs = [:]; let b = bend; bend = false
    lock.unlock()
    let t = Int(Date().timeIntervalSince(t0))
    if n.isEmpty && c.isEmpty && !b {
        quiet += 3
        if quiet == 12 { print("[\(t)s] quiet"); fflush(stdout) }
        if quiet % 15 == 0 { connectAll() }
        return
    }
    quiet = 0
    let seq = n.map { String(format: "%@@%.1f", nm($0.0), $0.2) }.joined(separator: " ")
    let vel = n.isEmpty ? 0 : n.map { Int($0.1) }.reduce(0, +) / n.count
    var line = "[\(t)s] \(n.count) notes vel~\(vel): \(seq)"
    if !h.isEmpty { line += " | held: \(h.sorted().map(nm).joined(separator: " "))" }
    if !c.isEmpty { line += " | cc: \(c.map { "\($0.key)=\($0.value)" }.sorted().joined(separator: " "))" }
    if b { line += " | bend" }
    print(line); fflush(stdout)
}
let timer = Timer(timeInterval: 3, repeats: true) { _ in summarize() }
RunLoop.current.add(timer, forMode: .default)
RunLoop.current.run()
