#!/usr/bin/env python3
"""Kite Weather — music video.

A lone walk at dusk down a power-line right-of-way: a cleared corridor
through dark forest, lattice pylons receding in a line, the wires slung
between them. Everything is raymarched on the GPU and driven by the song:

  footsteps     the camera takes one step a beat; it slows under the line,
                stops beneath a pylon and looks up into the wires, holds
                there through the silent bar, walks on, and at the end
                cranes up to show the line running to the horizon
  the chords    their swells light the sky and the air; each chord moves
                the horizon's colour
  the wires     glow with the lines' hum and the tuned wind singing in them
  the sparks    every spark in the audio is corona discharge at a real
                insulator on the pylon ahead — its side from the spark's
                pan, its arm from its pitch
  the texture   the chord texture drifts through the air as motes
  the bass      stirs the ground mist; the drums give the walk its weight

No text. Nothing flashes the frame: the sparks are points with bloom.

    python scripts/kite-weather-video.py --audio mix.wav --stems DIR \\
        --events events.json --out renders/kite-weather.mp4

Needs numpy, soundfile, moderngl (and pillow for stills); ffmpeg on PATH.
"""
import argparse, json, subprocess
import numpy as np, soundfile as sf
import moderngl

ap = argparse.ArgumentParser()
ap.add_argument('--audio', required=True)
ap.add_argument('--stems', required=True)
ap.add_argument('--events', required=True)
ap.add_argument('--out', required=True)
ap.add_argument('--fps', type=int, default=30)
ap.add_argument('--width', type=int, default=1920)
ap.add_argument('--height', type=int, default=1080)
ap.add_argument('--stills', default='', help='comma-separated seconds: write PNGs instead of a video')
A = ap.parse_args()

FPS, W, H = A.fps, A.width, A.height
B = 60 / 76                                   # seconds per beat
DUR = sf.info(A.audio).duration
N = int(DUR * FPS)
T = np.arange(N) / FPS
BEAT = T / B


def smooth(x, att, rel):
    out, v = np.zeros_like(x), 0.0
    a, r = 1 - np.exp(-1 / FPS / att), 1 - np.exp(-1 / FPS / rel)
    for i, s in enumerate(x):
        v += (s - v) * (a if s > v else r)
        out[i] = v
    return out


def stem_power(name):
    x, sr = sf.read(f'{A.stems}/{name}.wav', dtype='float32', always_2d=True)
    m = (x ** 2).mean(1)
    hop = sr / FPS
    edges = (np.arange(N + 1) * hop).astype(int).clip(0, len(m))
    sums = np.add.reduceat(m, edges[:-1])[:N]
    counts = np.maximum(np.diff(edges), 1)[:N]
    return sums / counts


def env(names, att, rel):
    p = sum(stem_power(n) for n in names)
    e = smooth(np.sqrt(p), att, rel)
    return e / (np.percentile(e, 98) + 1e-9)


pad = env(['low', 'mid', 'top'], 0.4, 1.2)
flow = env(['flow1', 'flow2', 'flow3'], 0.2, 0.8)
wires = env(['wire1', 'wire2', 'wire3', 'wire4'], 0.3, 1.0)
hum = env(['hum'], 0.5, 1.5)
bass = env(['bass'], 0.1, 0.6)
drums = env(['break', 'tom'], 0.01, 0.18)

# --- the score
ev = json.load(open(A.events))
tr = ev['tracks']
crk = tr['crackle']
pans = sorted((e['t'], e['value']) for e in ev['events']
              if e['type'] == 'param' and e['ch'] == crk and e['param'] == 'TrackPan')
sparks = []
for e in ev['events']:
    if e['type'] == 'on' and e['ch'] == crk:
        pan = next((v for t, v in reversed(pans) if t <= e['t'] + 0.03), 0.0)
        sparks.append((e['t'], e['note'], e['vel'], pan))

# --- the walk: speed by beat, integrated; the camera stops under a pylon
V = 1.35
spd = np.full(N, V)
spd[BEAT < 16] += 1.3 * np.clip(1 - BEAT[BEAT < 16] / 16, 0, 1) ** 0.5   # the intro glides in
def ramp(b0, b1, v0, v1):
    m = (BEAT >= b0) & (BEAT < b1)
    u = (BEAT[m] - b0) / (b1 - b0)
    spd[m] = v0 + (v1 - v0) * (u * u * (3 - 2 * u))
ramp(48, 54, V, 0.0)
spd[(BEAT >= 54) & (BEAT < 64)] = 0.0
ramp(64, 68, 0.0, V)
ramp(102, 112, V, 0.25)
spd[BEAT >= 112] = 0.25
Z = np.cumsum(spd) / FPS
S = 40.0                                       # pylon spacing
z_stop = Z[np.searchsorted(BEAT, 56)]
PHASE = z_stop % S                             # a pylon stands exactly where the walk stops

walking = np.clip(spd / V, 0, 1)
rise = np.clip(BEAT / 14.0, 0, 1); rise = rise * rise * (3 - 2 * rise)   # the opening: from the grass up to eye height
bob = 0.045 * np.abs(np.sin(np.pi * BEAT)) * walking
sway = 0.06 * np.sin(np.pi * BEAT * 0.5) * walking
shake = 0.012 * drums * walking
def curve(b0, b1, v0, v1, x):
    u = np.clip((BEAT - b0) / (b1 - b0), 0, 1)
    return x + (v1 - v0) * (u * u * (3 - 2 * u))
pitch = 0.12 + 0.2 * (1 - rise)
pitch = curve(49, 56, 0.0, 0.92, pitch)        # looking up into the wires
pitch = curve(64, 68.5, 0.0, -0.92, pitch)
crane = curve(100, 116, 0.0, 1.0, np.zeros(N))
pitch = pitch - 0.3 * crane
cam_y = 0.32 + 1.33 * rise + bob + 11.0 * crane ** 1.4      # the film opens low in the grass and rises
cam_x = sway + 1.2 * np.sin(T * 0.045 + 0.6) * (1 - crane)   # within the pylons' legs (+-2.1 m at eye height): under each one, walk between them
yaw = (0.26 * np.sin(T * 0.031 + 1.0) + 0.09 * np.sin(T * 0.083)) * walking * (1 - crane) + 0.02 * np.sin(T * 0.19)

# --- light: chord colour, night, fade
PAL = np.array([[0.36, 0.40, 0.95],            # Em9: blue-violet
                [1.00, 0.62, 0.34],            # Cmaj7#11: amber
                [0.30, 0.78, 0.74],            # Am9: teal
                [0.95, 0.46, 0.58]])           # Bm11: rose
bar = np.floor(BEAT / 4).astype(int)
col = PAL[bar % 4]
for i in range(1, N):
    col[i] = col[i - 1] + (col[i] - col[i - 1]) * (1 - np.exp(-1 / FPS / 1.6))
night = np.interp(BEAT, [0, 16, 44, 52, 60, 64, 72, 96, 120], [0.78, 0.68, 0.7, 0.9, 1.0, 0.85, 0.55, 0.5, 0.62])
fade = np.clip(T / 4.0, 0, 1) ** 1.5 * np.clip((DUR - T) / 5.0, 0, 1) ** 1.2

MAXS = 8
def active_sparks(i):
    t, cz = T[i], Z[i]
    out = []
    for (t0, note, vel, pan) in sparks:
        dt = t - t0
        if dt < 0 or dt > 0.45:
            continue
        if walking[i] < 0.3:                    # standing under the line: the pylon overhead
            k = np.round((cz - PHASE) / S)
        else:                                   # walking: the next pylon ahead
            k = np.ceil((cz - PHASE + 4.0) / S)
        tz = PHASE + k * S
        high = note >= 53
        x = (3.8 if high else 5.8) * (1 if pan >= 0 else -1)
        y = 16.3 if high else 13.8
        flick = 0.6 + 0.4 * np.sin(97.0 * t + 13.0 * t0) * np.sin(61.0 * t)
        out.append((x, y, tz, vel * np.exp(-dt / 0.12) * flick))
    out = sorted(out, key=lambda s: -s[3])[:MAXS]
    return out + [(0, -100, 0, 0)] * (MAXS - len(out))

ctx = moderngl.create_standalone_context(require=330)
VS = """
#version 330
in vec2 p; out vec2 uv;
void main(){ uv = p*0.5+0.5; gl_Position = vec4(p,0,1); }
"""
SCENE = """
#version 330
in vec2 uv; out vec4 o;
uniform vec2 R; uniform float t, pad, flow, wires, hum, bass, drums, night, fade, PH, SP;
uniform vec3 ro, fwd, upv, pal;
uniform vec4 spk[8];

const vec3 MOON = normalize(vec3(-0.38, 0.22, 0.9));
float h2(vec2 p){ p = fract(p*vec2(123.34, 456.21)); p += dot(p, p+45.32); return fract(p.x*p.y); }
float n2(vec2 x){ vec2 i = floor(x), f = fract(x); f = f*f*(3.0-2.0*f);
    return mix(mix(h2(i), h2(i+vec2(1,0)), f.x), mix(h2(i+vec2(0,1)), h2(i+vec2(1,1)), f.x), f.y); }
float fbm(vec2 p){ float a = 0.5, s = 0.0; for (int i = 0; i < 4; i++){ s += a*n2(p); p = p*2.03+vec2(1.7,9.2); a *= 0.5; } return s; }
float h3(vec3 p){ p = fract(p*0.3183099+0.1); p *= 17.0; return fract(p.x*p.y*p.z*(p.x+p.y+p.z)); }

float ground(vec2 xz){
    float h = 0.45*fbm(xz*0.06) + 0.12*fbm(xz*0.5);
    h *= mix(0.35, 1.0, smoothstep(2.5, 9.0, abs(xz.x)));
    h += 0.6*smoothstep(8.0, 16.0, abs(xz.x));
    // tufts of grass near the walker, fading out before they can shimmer
    float near = smoothstep(16.0, 3.0, length(xz - ro.xz));
    if (near > 0.0){
        float tuft = n2(xz*23.0 + vec2(sin(t*1.3 + xz.y)*0.6, 0.0));
        h += near*0.13*tuft*tuft*tuft*(0.6 + 0.8*n2(xz*3.0));
    }
    return h - 0.35;
}
float edgeX(float z){ return 9.5 + 1.6*n2(vec2(z*0.08, 3.1)); }
float trees(vec3 p){
    float cs = 2.6;
    vec2 c = floor(p.xz/cs);
    float d = 1e3;
    for (int j = -1; j <= 1; j++) for (int i = -1; i <= 1; i++){
        vec2 cc = c + vec2(i, j);
        vec2 ctr = (cc + 0.5 + 0.35*(vec2(h2(cc), h2(cc+7.1)) - 0.5))*cs;
        if (abs(ctr.x) < edgeX(ctr.y)) continue;
        float base = ground(ctr) - 0.2;
        float hh = 7.0 + 6.0*h2(cc+3.3) + 2.0*smoothstep(12.0, 24.0, abs(ctr.x));
        vec3 q = vec3(p.x - ctr.x, p.y - base, p.z - ctr.y);
        float r = 1.5*(0.35 + 0.65*h2(cc+1.7))*(1.0 - clamp(q.y/hh, 0.0, 1.0));
        r *= 0.78 + 0.22*abs(sin(q.y*2.6 + h2(cc)*6.0)) + 0.08*n2(vec2(atan(q.z, q.x)*3.0, q.y*2.0));   // tiers of branches
        r *= 0.86 + 0.28*n2(vec2(atan(q.z, q.x)*7.0 + h2(cc)*9.0, q.y*6.0));                         // ragged edges
        float cone = max(length(q.xz) - r, q.y - hh);
        d = min(d, max(cone, -q.y));
    }
    return d*0.8;
}
float cap(vec3 p, vec3 a, vec3 b, float r){ vec3 pa = p-a, ba = b-a; float h = clamp(dot(pa,ba)/dot(ba,ba), 0.0, 1.0); return length(pa-ba*h) - r; }
float legW(float y){ return mix(2.3, 0.65, y/19.0); }
float tower(vec3 p){
    vec3 q = p; q.z = mod(q.z - PH + 0.5*SP, SP) - 0.5*SP;
    vec3 bq = abs(q) - vec3(7.0, 11.0, 3.0); bq.y = abs(q.y - 10.5) - 11.0;
    float bb = length(max(bq, 0.0));
    if (bb > 1.0) return bb;
    q.x = abs(q.x); q.z = abs(q.z);
    float d = cap(q, vec3(2.3, 0.0, 2.3), vec3(0.65, 19.0, 0.65), 0.09);
    for (int k = 0; k < 6; k++){
        float y0 = float(k)*3.2, y1 = y0 + 3.2;
        d = min(d, cap(q, vec3(legW(y0), y0, legW(y0)), vec3(legW(y1), y1, 0.0), 0.035));
        d = min(d, cap(q, vec3(legW(y0), y0, legW(y0)), vec3(0.0, y1, legW(y1)), 0.035));
    }
    d = min(d, cap(q, vec3(0.0, 15.0, 0.0), vec3(6.0, 15.0, 0.0), 0.11));
    d = min(d, cap(q, vec3(legW(12.8), 12.8, 0.0), vec3(6.0, 15.0, 0.0), 0.05));
    d = min(d, cap(q, vec3(0.0, 17.5, 0.0), vec3(4.0, 17.5, 0.0), 0.1));
    d = min(d, cap(q, vec3(legW(15.8), 15.8, 0.0), vec3(4.0, 17.5, 0.0), 0.045));
    d = min(d, cap(q, vec3(0.0, 19.0, 0.0), vec3(0.0, 20.6, 0.0), 0.08));
    d = min(d, cap(q, vec3(5.8, 15.0, 0.0), vec3(5.8, 13.8, 0.0), 0.08));   // insulator strings
    d = min(d, cap(q, vec3(3.8, 17.5, 0.0), vec3(3.8, 16.3, 0.0), 0.08));
    return d;
}
float wy(float y0, float z){ float u = fract((z - PH)/SP); return y0 - 2.4*4.0*u*(1.0-u); }
float wireD(vec3 p, float x0, float y0){ return length(vec2(abs(p.x) - x0, p.y - wy(y0, p.z))); }
float wiresD(vec3 p){
    float d = wireD(p, 5.8, 13.8);
    d = min(d, wireD(p, 3.8, 16.3));
    d = min(d, length(vec2(p.x, p.y - wy(20.6, p.z))));
    return d;
}
float map(vec3 p, out int m){
    float g = p.y - ground(p.xz);
    float d = g*0.7; m = 0;
    float tr = trees(p); if (tr < d){ d = tr; m = 1; }
    float tw = tower(p); if (tw < d){ d = tw; m = 2; }
    float wr = wiresD(p) - 0.03; if (wr < d){ d = wr; m = 3; }
    return d;
}
vec3 sky(vec3 rd){
    float up = max(rd.y, 0.0);
    vec3 zen = mix(vec3(0.05, 0.08, 0.2), vec3(0.008, 0.012, 0.035), night);
    vec3 hor = mix(pal*0.75 + vec3(0.18, 0.12, 0.1), vec3(0.03, 0.035, 0.07), night*0.85);
    hor *= 0.75 + 0.65*pad;
    vec3 c = mix(hor, zen, pow(up, 0.42));
    float md = dot(rd, MOON);
    c += vec3(1.0, 0.95, 0.85)*(smoothstep(0.99925, 0.9995, md)*2.6 + exp(-(1.0-md)*60.0)*0.35 + exp(-(1.0-md)*7.0)*0.08);
    // the afterglow: a warm band low on the horizon in the chord's colour
    c += (pal*0.9 + vec3(0.25, 0.1, 0.05))*exp(-up*9.0)*(0.35 + 0.45*pad)*(1.0 - 0.8*night);
    // two layers of cloud, lit by the moon: silver edges toward it, dark bellies
    for (int L = 0; L < 2; L++){
        float hgt = L == 0 ? 0.1 : 0.22;
        vec2 cl = rd.xz/(rd.y + hgt)*(L == 0 ? 1.1 : 0.7) + vec2(t*(0.012 + 0.006*float(L)), float(L)*3.7);
        float dens = fbm(cl*1.6) + 0.5*fbm(cl*4.3) - (L == 0 ? 0.62 : 0.72);
        float cov = smoothstep(0.0, 0.35, dens)*smoothstep(0.0, 0.25, up);
        float lit = clamp(0.5 + 3.0*(fbm(cl*1.6 + MOON.xz*0.05) - fbm(cl*1.6)), 0.0, 1.0);
        vec3 ccol = mix(hor*0.35 + vec3(0.01), vec3(0.75, 0.78, 0.9)*(0.35 + 0.65*(1.0 - night*0.6)), lit*0.6);
        ccol += vec3(1.0, 0.95, 0.85)*pow(max(md, 0.0), 40.0)*0.6*(1.0 - smoothstep(0.1, 0.4, dens));   // silver lining
        c = mix(c, ccol, cov*0.8);
    }
    vec2 sg = floor(rd.xz/(rd.y + 0.3)*260.0);
    float star = step(0.9985, h2(sg))*smoothstep(0.05, 0.4, up)*smoothstep(0.5, 0.95, night);
    c += star*vec3(0.8, 0.85, 1.0)*(0.6 + 0.4*sin(t*3.0 + h2(sg+1.0)*40.0));
    return c;
}
vec3 normalAt(vec3 p){
    int m; vec2 e = vec2(0.01, 0.0);
    return normalize(vec3(map(p+e.xyy, m) - map(p-e.xyy, m), map(p+e.yxy, m) - map(p-e.yxy, m), map(p+e.yyx, m) - map(p-e.yyx, m)));
}
void main(){
    vec2 p = (uv*R - 0.5*R)/R.y;
    vec3 rt = normalize(cross(fwd, upv)), uu = cross(rt, fwd);
    vec3 rd = normalize(fwd*1.25 + rt*p.x + uu*p.y);
    float tt = 0.05; int m = -1; float glow = 0.0;
    for (int i = 0; i < 220; i++){
        vec3 q = ro + rd*tt;
        int mm; float d = map(q, mm);
        float wd = wiresD(q);
        float foot = 0.0015*tt + 0.004;
        glow += exp(-wd*wd/(foot*foot*4.0))*min(abs(d), 0.4)/(foot*6.0);
        if (d < 0.0008*tt){ m = mm; break; }
        tt += d;
        if (tt > 420.0) break;
    }
    vec3 amb = sky(vec3(0.0, 1.0, 0.0))*0.6 + sky(normalize(vec3(rd.x, 0.05, rd.z)))*0.4;
    vec3 c;
    if (m < 0){
        c = sky(rd);
        tt = 420.0;
    } else {
        vec3 q = ro + rd*tt; vec3 n = normalAt(q);
        float dif = max(dot(n, MOON), 0.0);
        float grass = fbm(q.xz*1.7)*0.6 + 0.4*n2(q.xz*9.0);
        vec3 alb = m == 0 ? vec3(0.13, 0.15, 0.1)*(0.55 + 0.9*grass) :
                   m == 1 ? vec3(0.025, 0.04, 0.035) :
                   m == 2 ? vec3(0.22, 0.23, 0.26) : vec3(0.015);
        float sh = 1.0; { int mm; float s = 0.3; for (int k = 0; k < 16; k++){ float h = map(q + n*0.02 + MOON*s, mm); sh = min(sh, 10.0*h/s); s += clamp(h, 0.2, 3.0); if (sh < 0.01 || s > 40.0) break; } sh = clamp(sh, 0.0, 1.0); }
        c = alb*(vec3(0.75, 0.78, 0.9)*dif*sh*(0.55 + 0.35*(1.0 - night)) + amb*(0.7 + 0.5*n.y));
        if (m == 0) c += vec3(0.6, 0.65, 0.8)*pow(max(dot(reflect(rd, n), MOON), 0.0), 24.0)*0.05*grass;   // dew catching the moon
        // wire glow lights the steel and the grass beneath it a little
        c += alb*vec3(0.5, 0.65, 1.0)*(0.15*wires + 0.1*hum)*exp(-abs(q.y - 14.0)*0.08)*(m == 2 ? 2.0 : 0.3);
        // the sparks light the steel around them
        for (int k = 0; k < 8; k++){
            if (spk[k].w <= 0.0) continue;
            vec3 L = spk[k].xyz - q; float d2 = dot(L, L);
            c += alb*vec3(0.7, 0.8, 1.0)*spk[k].w*14.0*max(dot(n, L*inversesqrt(d2)), 0.2)/(1.0 + d2);
        }
    }
    // height fog, lit by the sky's horizon and the moon
    // height fog, integrated along the ray for any direction (up OR down: the
    // old clamp starved downward rays, so far ground stayed dark under bright,
    // fogged trees and drew a hard line across the frame)
    float fd = 0.0095*(1.0 + 0.6*bass);
    vec3 fogC = sky(normalize(vec3(rd.x, 0.02, rd.z)))*0.9;
    // beyond the volume: the analytic height-fog integral (valid up and down)
    float VOL = 90.0;
    if (tt > VOL){
        float kf = rd.y*0.12;
        float e0 = exp(-(ro.y + rd.y*VOL)*0.12);
        float path = abs(kf) > 1e-4 ? (1.0 - exp(-(tt - VOL)*kf))/kf : tt - VOL;
        float fogA = clamp(1.0 - exp(-fd*e0*path), 0.0, 1.0);
        c = mix(c, fogC, fogA);
    }
    // inside it: a real volume. Moonlight scattered forward through wisps of
    // mist, shadowed by the trees and the lattice — shafts through the steel —
    // the wires' glow spilling into the mist, and the sparks lighting it.
    {
        float tv = min(tt, VOL);
        const int NS = 44;
        float dt = tv/float(NS);
        float jit = fract(sin(dot(uv*R, vec2(12.9898, 78.233)) + t)*43758.5453);
        float cosT = dot(rd, MOON);
        float g = 0.72;
        float phase = 0.04 + 1.6*(1.0 - g*g)/pow(1.0 + g*g - 2.0*g*cosT, 1.5)/(4.0*3.14159);
        vec3 moonC = vec3(0.7, 0.75, 0.9)*(0.55 + 0.45*(1.0 - night));
        vec3 wcolV = mix(vec3(0.55, 0.7, 1.0), pal, 0.3);
        float Tr = 1.0; vec3 acc = vec3(0.0);
        for (int k = 0; k < NS; k++){
            float s = (float(k) + jit)*dt;
            vec3 x = ro + rd*s;
            float wisp = fbm(x.xz*0.09 + vec2(t*0.03, x.y*0.07));
            float dens = 0.85*fd*exp(-max(x.y, 0.0)*0.14)*(0.2 + 1.6*wisp*wisp);
            // shadow toward the moon: trees and steel only
            float sh = 1.0, sm = 0.4;
            for (int j = 0; j < 6; j++){
                vec3 y = x + MOON*sm;
                float hd = min(trees(y), tower(y));
                sh = min(sh, clamp(6.0*hd/sm, 0.0, 1.0));
                sm += max(hd, 0.6);
                if (sh < 0.02 || sm > 30.0) break;
            }
            vec3 L = moonC*1.7*phase*sh + fogC*0.1;
            float wd = wiresD(x);
            L += wcolV*(0.2*wires + 0.12*hum)*exp(-wd*wd*1.2)*6.0;
            for (int q = 0; q < 8; q++){
                if (spk[q].w <= 0.0) continue;
                vec3 dv = spk[q].xyz - x;
                L += vec3(0.7, 0.8, 1.0)*spk[q].w*30.0/(1.0 + dot(dv, dv)*0.8);
            }
            acc += Tr*dens*L*dt;
            Tr *= exp(-dens*dt);
        }
        c = c*Tr + acc*1.5;
    }
    // ground mist, stirred by the bass
    if (rd.y < 0.02 || ro.y < 1.5){
        float mist = 0.0;
        for (int k = 1; k <= 6; k++){
            float s = float(k)*float(k)*1.1;
            if (s > tt) break;
            vec3 q = ro + rd*s;
            mist += smoothstep(0.9, -0.2, q.y - ground(q.xz))*fbm(q.xz*0.3 + vec2(t*0.08, 0.0))*0.09;
        }
        c = mix(c, fogC*1.1, clamp(mist*(0.7 + 0.8*bass), 0.0, 0.55));
    }
    // the lines glowing: the hum and the wind singing in the wires
    vec3 wcol = mix(vec3(0.55, 0.7, 1.0), pal, 0.25);
    c += wcol*glow*(0.012 + 0.14*wires + 0.09*hum)*(0.5 + 0.5*night);
    // corona sparks at the insulators ahead
    for (int k = 0; k < 8; k++){
        vec3 sp = spk[k].xyz - ro; float I = spk[k].w;
        if (I <= 0.0) continue;
        float along = dot(sp, rd);
        if (along < 0.0 || along > tt + 0.5) continue;
        float dd = length(sp - rd*along);
        float ang = dd/along;
        c += vec3(0.75, 0.85, 1.0)*I*(exp(-ang*ang*6e4)*9.0 + exp(-ang*ang*4e3)*0.9 + exp(-ang*ang*300.0)*0.08);
    }
    o = vec4(c*fade, 1.0);
}
"""
POST = """
#version 330
in vec2 uv; out vec4 o;
uniform sampler2D S; uniform vec2 R; uniform float t, flow, spark, fade, drums;
uniform vec3 pal;
float hash(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7)))*43758.5453); }
vec3 aces(vec3 x){ return clamp((x*(2.51*x+0.03))/(x*(2.43*x+0.59)+0.14), 0.0, 1.0); }
void main(){
    vec2 u = uv;
    vec3 c = textureLod(S, u, 1.0).rgb;          // the 2x scene, box-filtered down
    vec3 b = textureLod(S, u, 3.0).rgb*0.3 + textureLod(S, u, 4.5).rgb*0.3 + textureLod(S, u, 6.0).rgb*0.25 + textureLod(S, u, 7.5).rgb*0.2;
    c += b*0.45;
    // a lens streak through the sparks
    vec3 st = vec3(0.0);
    for (int k = -7; k <= 7; k++) st += textureLod(S, vec2(u.x + float(k)*0.035, u.y), 4.0).rgb*exp(-abs(float(k))*0.3);
    c += st*vec3(0.6, 0.75, 1.0)*0.05*spark;
    // motes: the chord texture drifting through the air, three depths
    float mote = 0.0;
    for (int l = 0; l < 3; l++){
        float sc = 40.0 + 45.0*float(l);
        vec2 g = u*vec2(R.x/R.y, 1.0)*sc + vec2(t*(0.2 + 0.1*float(l)), -t*(0.12 + 0.05*float(l)));
        vec2 id = floor(g), f = fract(g) - 0.5;
        float hh = hash(id + float(l)*13.0);
        vec2 off = vec2(hash(id+2.3) - 0.5, hash(id+5.9) - 0.5)*0.6;
        mote += step(0.955, hh)*smoothstep(0.1, 0.0, length(f - off))*(0.5 + 0.5*sin(t*1.7 + hh*50.0))*(0.4 + 0.3*float(l));
    }
    c += mix(vec3(1.0, 0.85, 0.6), pal, 0.4)*mote*flow*0.35*fade;
    c = aces(c*1.35);
    c *= smoothstep(1.3, 0.3, length((u - 0.5)*vec2(1.25, 1.0)));
    c += (hash(u*R + fract(t*13.0)*100.0) - 0.5)*0.022;
    float box = 0.5*(R.x/R.y)/2.35;
    c *= step(abs(u.y - 0.5), box);
    o = vec4(pow(max(c, 0.0), vec3(1.0/1.05)), 1.0);
}
"""
quad = ctx.buffer(np.array([-1, -1, 1, -1, -1, 1, 1, 1], dtype='f4').tobytes())
sp = ctx.program(vertex_shader=VS, fragment_shader=SCENE)
pp = ctx.program(vertex_shader=VS, fragment_shader=POST)
va_s = ctx.vertex_array(sp, [(quad, '2f', 'p')])
va_p = ctx.vertex_array(pp, [(quad, '2f', 'p')])
SS = 2
tex = ctx.texture((W * SS, H * SS), 4, dtype='f2')
tex.filter = (moderngl.LINEAR_MIPMAP_LINEAR, moderngl.LINEAR)
fbo_s = ctx.framebuffer(color_attachments=[tex])
fbo_o = ctx.framebuffer(color_attachments=[ctx.renderbuffer((W, H), 4)])


def setu(prog, **kw):
    for k, v in kw.items():
        if k in prog:
            prog[k].value = v


def frame(i):
    ro = np.array([cam_x[i], cam_y[i], Z[i]])
    fwd = np.array([np.sin(yaw[i]) * np.cos(pitch[i]), np.sin(pitch[i]), np.cos(yaw[i]) * np.cos(pitch[i])])
    fwd += np.array([shake[i] * np.sin(97 * T[i]), shake[i] * np.sin(71 * T[i]), 0])
    fwd /= np.linalg.norm(fwd)
    s = active_sparks(i)
    spark_total = float(sum(v[3] for v in s))
    common = dict(t=float(T[i]), R=(float(W * SS), float(H * SS)), fade=float(fade[i]), pal=tuple(float(c) for c in col[i]))
    fbo_s.use()
    ctx.viewport = (0, 0, W * SS, H * SS)
    setu(sp, pad=float(pad[i]), flow=float(flow[i]), wires=float(wires[i]), hum=float(hum[i]), bass=float(bass[i]),
         drums=float(drums[i]), night=float(night[i]), PH=float(PHASE), SP=float(S),
         ro=tuple(ro), fwd=tuple(fwd), upv=(0.0, 1.0, 0.0), **common)
    sp['spk'].write(np.array(s, dtype='f4').tobytes())
    va_s.render(moderngl.TRIANGLE_STRIP)
    tex.build_mipmaps()
    fbo_o.use()
    ctx.viewport = (0, 0, W, H)
    tex.use(0)
    setu(pp, S=0, flow=float(flow[i]), spark=spark_total, drums=float(drums[i]), **common)
    va_p.render(moderngl.TRIANGLE_STRIP)
    return fbo_o.read(components=3)


if A.stills:
    from PIL import Image
    for s_ in A.stills.split(','):
        i = min(int(float(s_) * FPS), N - 1)
        Image.frombytes('RGB', (W, H), frame(i)).transpose(Image.FLIP_TOP_BOTTOM).save(f'{A.out}_{float(s_):05.1f}s.png')
    raise SystemExit

ff = subprocess.Popen([
    'ffmpeg', '-y', '-loglevel', 'error',
    '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-s', f'{W}x{H}', '-r', str(FPS), '-i', '-',
    '-i', A.audio,
    '-vf', 'vflip', '-c:v', 'libx264', '-preset', 'slow', '-crf', '15', '-pix_fmt', 'yuv420p',
    '-c:a', 'aac', '-b:a', '320k', '-shortest', '-movflags', '+faststart', A.out,
], stdin=subprocess.PIPE)
for i in range(N):
    ff.stdin.write(frame(i))
    if i % (FPS * 10) == 0:
        print(f'{T[i]:5.1f}s / {DUR:.1f}s', flush=True)
ff.stdin.close()
ff.wait()
print('wrote', A.out)
