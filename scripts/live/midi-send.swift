// Persistent virtual MIDI source "Claude", fed by a FIFO:
//   cc <num> <val>   pc <program>   note <ch> <note> <vel>   off <ch> <note>   ramp <cc> <from> <to> <ms>
// Build: swiftc -O midi-send.swift -o midi-send ; run: ./midi-send /path/to/fifo
import CoreMIDI
import Foundation
var client = MIDIClientRef()
MIDIClientCreateWithBlock("claude" as CFString, &client) { _ in }
var src = MIDIEndpointRef()
MIDISourceCreateWithProtocol(client, "Claude" as CFString, ._1_0, &src)
func send(_ b0: UInt32, _ b1: UInt32, _ b2: UInt32) {
    var list = MIDIEventList()
    let pkt = MIDIEventListInit(&list, ._1_0)
    var word: UInt32 = (0x20 << 24) | (b0 << 16) | (b1 << 8) | b2
    MIDIEventListAdd(&list, MemoryLayout<MIDIEventList>.size, pkt, 0, 1, &word)
    MIDIReceivedEventList(src, &list)
}
let fifo = CommandLine.arguments[1]
print("Claude source up, fifo \(fifo)"); fflush(stdout)
while true {
    guard let fh = FileHandle(forReadingAtPath: fifo) else { sleep(1); continue }
    let data = fh.readDataToEndOfFile(); fh.closeFile()
    for line in String(decoding: data, as: UTF8.self).split(separator: "\n") {
        let p = line.split(separator: " ").map(String.init)
        guard !p.isEmpty else { continue }
        func n(_ i: Int) -> UInt32 { UInt32(p.count > i ? (Int(p[i]) ?? 0) : 0) }
        switch p[0] {
        case "cc": send(0xB0, n(1), n(2))
        case "pc": send(0xC0, n(1), 0)
        case "note": send(0x90 | (n(1) & 0xF), n(2), n(3))
        case "off": send(0x80 | (n(1) & 0xF), n(2), 0)
        case "ramp":
            let cc = n(1), a = Double(n(2)), b = Double(n(3)), ms = Double(n(4))
            let steps = max(1, Int(ms / 40))
            for s in 0...steps {
                let v = a + (b - a) * Double(s) / Double(steps)
                send(0xB0, cc, UInt32(max(0, min(127, v.rounded()))))
                usleep(useconds_t(ms / Double(steps) * 1000))
            }
        default: break
        }
        print("sent: \(line)"); fflush(stdout)
    }
}
