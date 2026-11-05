//! The `patina feel` physics on its own: each venue's transfer function
//! (sound pressure at 1 m to vertical acceleration at the body), the
//! whole chain at full scale, the ISO 2631-1 Wk check, and how long each
//! floor rings. No rendering, no files.
//!
//!   cargo run --release --example feel_venues
use patina::feel::{self, Field, Slab, Timber, Venue, WK_TABLE};

fn main() {
    println!("ISO 2631-1 Wk: this filter against the standard's Table 3");
    println!("{:>8} {:>8} {:>8} {:>7}", "Hz", "table", "filter", "error");
    for (f, table) in WK_TABLE {
        let g = feel::wk().gain(f);
        println!(
            "{:>8} {:>8.4} {:>8.4} {:>+6.2}%",
            f,
            table,
            g,
            100.0 * (g / table - 1.0)
        );
    }
    // A 1 m/s² peak sine at 50 Hz through the digital filter at 8 kHz:
    // its Wk r.m.s. should be 0.707 x the weighting there
    let fs = 8000.0;
    let mut w = feel::wk().run(fs);
    let y: Vec<f64> = (0..(fs * 4.0) as usize)
        .map(|i| w.process((std::f64::consts::TAU * 50.0 * i as f64 / fs).sin()))
        .collect();
    let tail = &y[(fs * 2.0) as usize..];
    let rms = (tail.iter().map(|v| v * v).sum::<f64>() / tail.len() as f64).sqrt();
    let expect = std::f64::consts::FRAC_1_SQRT_2 * feel::wk().gain(50.0);
    println!(
        "\n50 Hz sine, 1 m/s² peak: Wk r.m.s. {:.4} m/s² (expected {:.4}, {:+.2}%)",
        rms,
        expect,
        100.0 * (rms / expect - 1.0)
    );
    let freqs = [20.0, 25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0];
    for venue in [
        Venue::Field(Field::default()),
        Venue::Club(Slab::default()),
        Venue::House(Timber::default()),
    ] {
        let c = venue.coupling();
        let rig = venue.default_rig();
        let chain = rig.filter().then(c.filter());
        let pa = rig.full_scale_pa();
        println!("\n{}: {}", venue.name(), c.describe);
        println!(
            "  static {:.3e} m/Pa, f0 {:.1} Hz, Q {:.2}, rings {:.0} ms to -20 dB",
            c.static_m_per_pa,
            c.f0,
            c.q,
            c.ring_20db() * 1000.0
        );
        println!(
            "  {} level {:.0} dB peak at 1 m = {:.0} Pa; ground load {:.0} N at full scale",
            rig.level_name,
            rig.spl_db,
            pa,
            pa * c.load_n_per_pa
        );
        println!(
            "  {:>6} {:>14} {:>14} {:>14}",
            "Hz", "(m/s²)/Pa", "full-scale m/s²", "Wk m/s² peak"
        );
        for f in freqs {
            println!(
                "  {:>6} {:>14.3e} {:>14.4} {:>14.4}",
                f,
                c.filter().gain(f),
                chain.gain(f) * pa,
                chain.gain(f) * feel::wk().gain(f) * pa
            );
        }
    }
}
