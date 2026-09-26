#!/usr/bin/env python3
"""The Last Warm Room — music video.

One warm lamp in a fogged, dark room, lit by the song itself: the heartbeat
pulses the light, the ghost voice is a luminous thread that slides with its
glides, the tape's wow warps the space, the pad's downward sag sinks and
cools the room, and the germanium break fractures the light into burning
grain and split colour. At the cut there is only an ember and one heartbeat.
No text anywhere.

Everything is driven by the song's own score (`--export-events`) and its
stems (`--render-stems`), so the picture moves with the music, not beside it.

    python scripts/last-warm-room-video.py --audio mix.wav --stems DIR \
        --events events.json --out renders/last-warm-room.mp4

Needs numpy, soundfile, moderngl; ffmpeg on PATH.
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
mix, SR = sf.read(A.audio, always_2d=True)
DUR = len(mix) / SR
N = int(DUR * FPS)
T = np.arange(N) / FPS

# ---------------------------------------------------------------- the score
ev = json.load(open(A.events))
tracks = ev['tracks']
HEART, PAD, GHOST = tracks['heart'], tracks['pad'], tracks['ghost']


def curve(param, ch=0, default=0.0):
    pts = [(e['t'], e['value']) for e in ev['events']
           if e['type'] == 'param' and e['param'] == param and e['ch'] == ch]
    if not pts:
        return np.full(N, default)
    t, v = np.array(pts).T
    return np.interp(T, t, v)


fuzz = curve('FuzzAmount', 0, 0.3)
wow = curve('TapeWow', 0, 0.34)
sag = curve('PitchShift', PAD, 0.0)
wet = curve('ReverbWet', 0, 0.38)

ons = [e for e in ev['events'] if e['type'] == 'on']

# Heartbeat: a fast bloom and an exponential fall per hit, velocity-scaled.
heart = np.zeros(N)
for e in (e for e in ons if e['ch'] == HEART):
    dt = T - e['t']
    m = dt >= 0
    # a breath of light, not a strobe: ~50 ms bloom (the racing heart runs
    # at 4 Hz, over the 3-per-second large-flash guideline if it snapped)
    heart[m] += e['vel'] * (1 - np.exp(-dt[m] / 0.05)) * np.exp(-dt[m] / 0.28)

# Ghost pitch: follows its notes through the glide (RC, ~1.1 s portamento).
g_ons = sorted((e['t'], e['note']) for e in ons if e['ch'] == GHOST)
target = np.full(N, np.nan)
for (t0, n), nxt in zip(g_ons, g_ons[1:] + [(DUR, None)]):
    target[(T >= t0) & (T < nxt[0])] = n
target = np.where(np.isnan(target), np.nan, target)
first = np.nanmin(np.where(np.isnan(target), np.inf, T)) if g_ons else 0
filled = target.copy()
last = g_ons[0][1] if g_ons else 60
for i in range(N):
    if np.isnan(filled[i]):
        filled[i] = last
    last = filled[i]
ghost_pitch = np.zeros(N)
p = filled[0]
k = 1 - np.exp(-1 / FPS / 0.4)
for i in range(N):
    p += (filled[i] - p) * k
    ghost_pitch[i] = p
ghost_y = (ghost_pitch - 57) / 9          # ~A3..F4 -> 0..1

# Chords: each pad strike moves the room's colour to the next palette.
PALETTE = np.array([
    [1.00, 0.56, 0.26],   # Dm9: lamp amber
    [1.00, 0.46, 0.34],   # Bbmaj7#11: rose amber
    [0.58, 0.78, 0.70],   # Gm(maj7): the first cold light
    [0.66, 0.52, 0.92],   # Ebmaj7#11: violet
    [1.00, 0.30, 0.16],   # the cluster: burning
    [0.92, 0.72, 0.52],   # the return: ash amber
])
p_times = sorted({round(e['t'], 3) for e in ons if e['ch'] == PAD})
chord_idx = np.searchsorted(np.array(p_times), T, side='right') - 1
col = PALETTE[np.clip(chord_idx, 0, len(PALETTE) - 1)]
col[chord_idx < 0] = PALETTE[0]
# Glide the colour, slower into calm, faster into the break.
sm = col.copy()
for i in range(1, N):
    rate = 1 - np.exp(-1 / FPS / (0.35 if chord_idx[i] == 4 else 1.8))
    sm[i] = sm[i - 1] + (col[i] - sm[i - 1]) * rate
col = sm


# ---------------------------------------------------------------- the stems
def envelope(path, att=0.03, rel=0.35):
    x, sr = sf.read(path, always_2d=True)
    m = x.mean(1)
    hop = sr / FPS
    rms = np.array([np.sqrt(np.mean(m[int(i * hop):int((i + 1) * hop)] ** 2) + 1e-12) for i in range(N)])
    out = np.zeros(N)
    v = 0.0
    a, r = 1 - np.exp(-1 / FPS / att), 1 - np.exp(-1 / FPS / rel)
    for i in range(N):
        v += (rms[i] - v) * (a if rms[i] > v else r)
        out[i] = v
    return out / (np.percentile(out, 97) + 1e-9)


pad = envelope(f'{A.stems}/pad.wav', 0.2, 0.8)
drone = envelope(f'{A.stems}/drone.wav', 0.5, 1.2)
ghost = envelope(f'{A.stems}/ghost.wav', 0.15, 0.6)

# The cut (34-36 s): the instruments are pulled out from under the room.
cut = np.clip((T - 33.95) / 0.06, 0, 1) * np.clip((36.0 - T) / 0.5, 0, 1)
cut = np.maximum(cut, np.clip((T - 33.95) / 0.06, 0, 1) * (T < 36.0))
fade_in = np.clip(T / 5.0, 0, 1) ** 1.6
fade_out = np.clip((DUR - T) / 4.5, 0, 1) ** 1.3
fuzzN = np.clip((fuzz - 0.3) / 0.56, 0, 1)
wowN = np.clip((wow - 0.22) / 0.40, 0, 1)
sagN = np.clip(-sag / 0.7, 0, 1)

# ---------------------------------------------------------------- GPU
ctx = moderngl.create_standalone_context(require=330)
VS = """
#version 330
in vec2 p; out vec2 uv;
void main(){ uv = p*0.5+0.5; gl_Position = vec4(p,0,1); }
"""
SCENE = """
#version 330
in vec2 uv; out vec4 o;
uniform vec2 R; uniform float t, pad, drone, ghost, heart, fuzz, wow, sag, wet, cut, gy, fade;
uniform vec3 lamp;

float h3(vec3 p){ p = fract(p*0.3183099+0.1); p *= 17.0; return fract(p.x*p.y*p.z*(p.x+p.y+p.z)); }
float vn(vec3 x){
    vec3 i = floor(x), f = fract(x); f = f*f*f*(f*(f*6.0-15.0)+10.0);
    return mix(mix(mix(h3(i),h3(i+vec3(1,0,0)),f.x), mix(h3(i+vec3(0,1,0)),h3(i+vec3(1,1,0)),f.x), f.y),
               mix(mix(h3(i+vec3(0,0,1)),h3(i+vec3(1,0,1)),f.x), mix(h3(i+vec3(0,1,1)),h3(i+vec3(1,1,1)),f.x), f.y), f.z);
}
float fbm(vec3 p){ float a = 0.5, s = 0.0; for (int i = 0; i < 5; i++){ s += a*vn(p); p = p*2.03 + vec3(1.7,-0.9,2.3); a *= 0.5; } return s; }

vec3 warp(vec3 p){
    // the tape's wow bends the room: slow, heavy, getting worse
    float w = 0.15 + 0.9*wow;
    p.x += w*0.55*sin(p.z*0.7 + t*0.9);
    p.y += w*0.25*sin(p.x*0.9 - t*0.6);
    return p;
}

float density(vec3 p){
    vec3 q = warp(p) + vec3(0.0, 0.0, -t*0.18);
    float d = fbm(q*0.62 + vec3(0.0, t*0.02, 0.0));
    float floorMist = exp(-max(p.y + 1.2, 0.0)*2.2);
    float f = smoothstep(0.46, 0.82, d)*1.6 + floorMist*0.35;
    // the germanium tears the fog into fine, burning grain
    float grain = abs(vn(q*5.5 + t*3.0) - 0.5)*2.0;
    f += fuzz*fuzz*smoothstep(0.55, 1.0, grain)*1.3;
    return f*(0.55 + 0.35*wet);
}

vec3 thread(float z){
    // the ghost's line through the fog: height is its pitch
    return vec3(0.55*sin(z*0.33 + t*0.22) - 0.15, -0.45 + 1.0*gy + 0.10*sin(z*0.9 - t*0.45), z);
}

void main(){
    vec2 p = (uv*R - 0.5*R)/R.y;
    // the room sinks and leans as the chords sag flat
    float roll = 0.05*sag*sin(t*0.3) + 0.02*sin(t*0.13);
    p = mat2(cos(roll), -sin(roll), sin(roll), cos(roll))*p;
    // a slow, unsteady drift toward the lamp; the camera sinks with the sag
    vec3 ro = vec3(0.35*sin(t*0.061) + 0.12*sin(t*0.23), 0.1 - 0.35*sag + 0.05*sin(t*0.17), -1.0 + t*0.06);
    vec3 rd = normalize(vec3(p, 1.35));
    rd.y -= 0.12*sag;
    rd = normalize(rd);

    vec3 L = vec3(0.25*sin(t*0.11), 0.35 - 0.7*sag, ro.z + 7.5);
    float pulse = 1.0 + 0.75*heart;
    float unease = wow*(1.0 - fuzz);
    float flick = 1.0 + fuzz*fuzz*0.22*(fract(sin(floor(t*24.0)*91.7)*4375.85) - 0.5);
    float power = (0.3 + 0.75*pad + 0.4*drone)*pulse*flick*mix(1.0, 0.55, unease)*(1.0 - 0.94*cut) + 0.35*heart*cut;
    vec3 lc = mix(lamp, vec3(1.0, 0.92, 0.85), 0.55*fuzz);

    float jitter = fract(sin(dot(uv*R, vec2(12.9898, 78.233)) + t*7.0)*43758.5453);
    float dt = 0.2, T = 1.0;
    vec3 acc = vec3(0.0);
    float s = 0.4 + dt*jitter;
    for (int i = 0; i < 64; i++){
        vec3 x = ro + rd*s;
        float d = density(x);
        vec3 toL = L - x; float r2 = dot(toL, toL);
        float att = power/(1.0 + 1.5*r2);
        float cosT = dot(rd, normalize(toL));
        float hg = (1.0 - 0.36)/pow(1.0 + 0.36 - 1.2*cosT, 1.5);   // forward scattering
        vec3 ld = normalize(toL);
        float sh = exp(-(density(x + ld*0.45) + density(x + ld*1.3))*0.8);
        vec3 inscat = lc*att*hg*sh;
        // the thread: an emissive filament inside the fog
        vec3 c = thread(x.z);
        float dth = length(x.xy - c.xy);
        float far = smoothstep(ro.z + 3.0, ro.z + 6.0, x.z);
        vec3 glow = mix(vec3(1.0, 0.85, 0.7), lamp, 0.4)*ghost*1.1*exp(-dth*dth/0.0009)*far*(1.0 - cut);
        acc += T*(d*inscat*dt*1.05 + glow*dt);
        T *= exp(-d*dt*0.55);
        s += dt;
        if (x.y < -1.25){
            // the floor: dark, faintly wet, holding a long reflection of the lamp
            float streak = exp(-pow((x.x - L.x)*2.6, 2.0))*exp(-abs(x.z - L.z)*0.35);
            float ripple = 0.55 + 0.9*vn(vec3(x.x*3.0, x.z*7.0 - t*0.8, t*0.3 + 2.0*wow*sin(x.x*2.0)));
            acc += T*lc*power*streak*ripple*0.35;
            T = 0.0;
            break;
        }
        if (T < 0.02) break;
    }
    // the lamp itself, seen through the fog (and the ember at the cut)
    vec3 lp = L - ro; float along = dot(lp, rd);
    float miss = length(lp - rd*along);
    float core = exp(-miss*miss/(0.012 + 0.03*heart*cut))*(power*1.4 + 0.25*cut*(0.4 + heart));
    acc += lc*core*T*1.2;
    o = vec4(acc*fade, 1.0);
}
"""
POST = """
#version 330
in vec2 uv; out vec4 o;
uniform sampler2D S; uniform vec2 R; uniform float t, fuzz, wow, heart, cut, sag, fade;
uniform vec3 lamp;
float hash(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7)))*43758.5453); }
vec3 aces(vec3 x){ return clamp((x*(2.51*x+0.03))/(x*(2.43*x+0.59)+0.14), 0.0, 1.0); }
vec3 scene(vec2 u){
    vec3 c = texture(S, u).rgb;
    vec3 b = vec3(0.0);
    b += textureLod(S, u, 2.0).rgb*0.30; b += textureLod(S, u, 3.5).rgb*0.28;
    b += textureLod(S, u, 5.0).rgb*0.24; b += textureLod(S, u, 6.5).rgb*0.20;
    return c + b*(0.55 + 0.3*heart);
}
void main(){
    vec2 u = uv;
    // tape: the picture breathes with the wow, and shivers at the break
    u.x += 0.0025*wow*sin(u.y*9.0 + t*2.2) + 0.0012*fuzz*fuzz*sin(u.y*140.0 + t*60.0);
    vec2 dir = (u - 0.5);
    float ca = 0.0008 + 0.02*fuzz*fuzz;
    vec3 c = vec3(scene(u - dir*ca).r, scene(u).g, scene(u + dir*ca).b);
    // dust: three depths of motes drifting down through the lamp light
    float dust = 0.0;
    for (int l = 0; l < 3; l++){
        float sc = 60.0 + 70.0*float(l);
        vec2 g = u*vec2(R.x/R.y, 1.0)*sc + vec2(0.0, t*(0.6 + 0.4*float(l)));
        vec2 id = floor(g), f = fract(g) - 0.5;
        float hsh = hash(id + float(l)*17.0);
        vec2 off = vec2(hash(id + 3.1) - 0.5, hash(id + 7.7) - 0.5)*0.6;
        float d = length(f - off);
        float tw = 0.5 + 0.5*sin(t*1.3 + hsh*40.0);
        dust += step(0.93, hsh)*smoothstep(0.08, 0.0, d)*tw*(0.35 + 0.25*float(l));
    }
    float lampNear = exp(-length((u - vec2(0.5, 0.56 - 0.12*sag))*vec2(1.6, 1.0))*3.0);
    c += lamp*dust*lampNear*(0.8 + heart)*(1.0 - 0.9*cut)*fade;
    // anamorphic streak through the lamp when the germanium breaks
    vec3 flare = vec3(0.0);
    for (int k = -6; k <= 6; k++){
        flare += textureLod(S, vec2(u.x + float(k)*0.045, u.y), 4.0).rgb*exp(-abs(float(k))*0.35);
    }
    c += flare*vec3(1.0, 0.55, 0.4)*0.22*fuzz*fuzz;
    c = aces(c*1.55);
    // cool and drain the colour as the room goes flat
    float lum = dot(c, vec3(0.299, 0.587, 0.114));
    c = mix(c, vec3(lum)*vec3(0.86, 0.95, 1.08), 0.55*sag);
    // vignette, grain, and the final crush
    c *= smoothstep(1.25, 0.25, length(dir*vec2(1.25, 1.0)));
    float gr = hash(u*R + fract(t*13.0)*100.0) - 0.5;
    c += gr*(0.018 + 0.14*fuzz*fuzz);
    c = pow(max(c, 0.0), vec3(1.0/1.08));
    o = vec4(c, 1.0);
}
"""
quad = ctx.buffer(np.array([-1, -1, 1, -1, -1, 1, 1, 1], dtype='f4').tobytes())
sp = ctx.program(vertex_shader=VS, fragment_shader=SCENE)
pp = ctx.program(vertex_shader=VS, fragment_shader=POST)
va_s = ctx.vertex_array(sp, [(quad, '2f', 'p')])
va_p = ctx.vertex_array(pp, [(quad, '2f', 'p')])
tex = ctx.texture((W, H), 4, dtype='f2')
tex.filter = (moderngl.LINEAR_MIPMAP_LINEAR, moderngl.LINEAR)
fbo_s = ctx.framebuffer(color_attachments=[tex])
out_rb = ctx.renderbuffer((W, H), 4)
fbo_o = ctx.framebuffer(color_attachments=[out_rb])


def setu(prog, **kw):
    for k, v in kw.items():
        if k in prog:
            prog[k].value = v


def frame(i):
    fade = float(fade_in[i] * fade_out[i])
    common = dict(t=float(T[i]), R=(float(W), float(H)), heart=float(heart[i]), fuzz=float(fuzzN[i]),
                  wow=float(wowN[i]), sag=float(sagN[i]), cut=float(cut[i]), fade=fade,
                  lamp=tuple(float(c) for c in col[i]))
    fbo_s.use()
    setu(sp, pad=float(pad[i]), drone=float(drone[i]), ghost=float(ghost[i]),
         wet=float((wet[i] - 0.38) / 0.24), gy=float(ghost_y[i]), **common)
    va_s.render(moderngl.TRIANGLE_STRIP)
    tex.build_mipmaps()
    fbo_o.use()
    tex.use(0)
    setu(pp, S=0, **common)
    va_p.render(moderngl.TRIANGLE_STRIP)
    return fbo_o.read(components=3)


if A.stills:
    from PIL import Image
    for s in A.stills.split(','):
        i = min(int(float(s) * FPS), N - 1)
        Image.frombytes('RGB', (W, H), frame(i)).transpose(Image.FLIP_TOP_BOTTOM).save(f'{A.out}_{float(s):05.1f}s.png')
    raise SystemExit

ff = subprocess.Popen([
    'ffmpeg', '-y', '-loglevel', 'error',
    '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-s', f'{W}x{H}', '-r', str(FPS), '-i', '-',
    '-i', A.audio,
    '-vf', 'vflip', '-c:v', 'libx264', '-preset', 'slow', '-crf', '14', '-pix_fmt', 'yuv420p',
    '-c:a', 'aac', '-b:a', '320k', '-shortest', '-movflags', '+faststart', A.out,
], stdin=subprocess.PIPE)
for i in range(N):
    ff.stdin.write(frame(i))
    if i % (FPS * 5) == 0:
        print(f'{T[i]:5.1f}s / {DUR:.1f}s', flush=True)
ff.stdin.close()
ff.wait()
print('wrote', A.out)
