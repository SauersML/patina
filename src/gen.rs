// src/gen.rs
//
// Generative automation: a lane segment whose values come from an
// expression, evaluated at control rate (song.rs `{expr}:D` tokens).
//
// An expression reads the clock and computes a value:
//
//   variables   t  song time, seconds (through the tempo lane)
//               b  song position, beats
//               s  beats since the segment began, p its progress 0..1
//               v  the lane's value when the segment began
//   constants   pi  tau  e
//   operators   + - * / % ^ (power, right to left), unary -, comparisons
//               < <= > >= == != (1 or 0), && || !, and c ? a : b
//   math        sin cos tan asin acos atan atan2(y,x) sinh cosh tanh exp
//               exp2 log (natural) log2 log10 sqrt abs sign floor ceil
//               round fract min(a,b,..) max(a,b,..) clamp(x,lo,hi)
//               mix(a,b,p) pow(a,b) smoothstep(e0,e1,x) snap(x,q)
//               mtof(note) (MIDI note to Hz) db(dB) (to a linear gain)
//   cycles      sine(x) tri(x) saw(x) square(x[,duty]): one period per
//               unit of x, -1..1 (sine(b/4) is one cycle every 4 beats)
//   steps       step(x) (1 from x = 0 on, 0 below)
//               seq(i, a, b, c, ..) (the list's entry floor(i), cycling)
//   noise       noise(x[,seed]) smooth gradient noise, about -1..1
//               rand(x[,seed]) a fresh uniform 0..1 for every integer x
//   chaos       lorenz(time[,axis[,rho[,sigma[,beta]]]])  the Lorenz
//               attractor (28, 10, 8/3), axis 0 x, 1 y, 2 z, about -1..1
//               rossler(time[,axis[,a[,b[,c]]]])  Rossler (0.2, 0.2, 5.7)
//               logistic(n, r[,x0])  x <- r x (1 - x), iterated floor(n)
//               times: 0..1, period doubling from r = 3, chaos past 3.57
//               henon(n[,axis[,a[,b]]])  the Henon map (1.4, 0.3)
//               kuramoto(time, k[,which[,n[,spread]]])  n (8) coupled
//               phase oscillators at 1 +- spread (0.5) cycles a time
//               unit, coupling k: which 0 the swarm's mean field (-1..1),
//               -1 its coherence (0 apart .. 1 locked), i its i-th voice
//               pendulum(time[,axis[,damping]])  a double pendulum (about
//               one swing a time unit): axis 0 sin of the upper arm's
//               angle, 1 the lower's, 2 the lower bob's sideways place
//
// The flows and maps are pure functions of their time (or count)
// argument: each call site integrates its system from time 0, with fixed
// steps, as far as it is asked, so `lorenz(t)` at beat 64 is the same
// whatever came before it, and `lorenz(s)` starts over with each segment.
// A map or flow whose parameters move is stepped with the values of the
// moment; swept slowly through a period doubling, a map lingers on the
// old cycle (now unstable, but nothing nudges it off), so a lane that
// wants each regime clean steps r: logistic(b, seq(b / 8, 2.8, 3.2, 3.5)).

use std::fmt;

/// What an expression can read.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ctx {
    pub t: f64,
    pub b: f64,
    pub s: f64,
    pub p: f64,
    pub v: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Var {
    T,
    B,
    S,
    P,
    V,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Func {
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
    Sinh,
    Cosh,
    Tanh,
    Exp,
    Exp2,
    Log,
    Log2,
    Log10,
    Sqrt,
    Abs,
    Sign,
    Floor,
    Ceil,
    Round,
    Fract,
    Min,
    Max,
    Clamp,
    Mix,
    Pow,
    Smoothstep,
    Snap,
    Mtof,
    Db,
    Sine,
    Tri,
    Saw,
    Square,
    Step,
    Seq,
    Noise,
    Rand,
    Lorenz,
    Rossler,
    Logistic,
    Henon,
    Kuramoto,
    Pendulum,
}

impl Func {
    /// Name, fewest and most arguments (None = any number).
    fn table() -> &'static [(&'static str, Func, usize, Option<usize>)] {
        use Func::*;
        &[
            ("sin", Sin, 1, Some(1)),
            ("cos", Cos, 1, Some(1)),
            ("tan", Tan, 1, Some(1)),
            ("asin", Asin, 1, Some(1)),
            ("acos", Acos, 1, Some(1)),
            ("atan", Atan, 1, Some(1)),
            ("atan2", Atan2, 2, Some(2)),
            ("sinh", Sinh, 1, Some(1)),
            ("cosh", Cosh, 1, Some(1)),
            ("tanh", Tanh, 1, Some(1)),
            ("exp", Exp, 1, Some(1)),
            ("exp2", Exp2, 1, Some(1)),
            ("log", Log, 1, Some(1)),
            ("log2", Log2, 1, Some(1)),
            ("log10", Log10, 1, Some(1)),
            ("sqrt", Sqrt, 1, Some(1)),
            ("abs", Abs, 1, Some(1)),
            ("sign", Sign, 1, Some(1)),
            ("floor", Floor, 1, Some(1)),
            ("ceil", Ceil, 1, Some(1)),
            ("round", Round, 1, Some(1)),
            ("fract", Fract, 1, Some(1)),
            ("min", Min, 1, None),
            ("max", Max, 1, None),
            ("clamp", Clamp, 3, Some(3)),
            ("mix", Mix, 3, Some(3)),
            ("pow", Pow, 2, Some(2)),
            ("smoothstep", Smoothstep, 3, Some(3)),
            ("snap", Snap, 2, Some(2)),
            ("mtof", Mtof, 1, Some(1)),
            ("db", Db, 1, Some(1)),
            ("sine", Sine, 1, Some(1)),
            ("tri", Tri, 1, Some(1)),
            ("saw", Saw, 1, Some(1)),
            ("square", Square, 1, Some(2)),
            ("step", Step, 1, Some(1)),
            ("seq", Seq, 2, None),
            ("noise", Noise, 1, Some(2)),
            ("rand", Rand, 1, Some(2)),
            ("lorenz", Lorenz, 1, Some(5)),
            ("rossler", Rossler, 1, Some(5)),
            ("logistic", Logistic, 2, Some(3)),
            ("henon", Henon, 1, Some(4)),
            ("kuramoto", Kuramoto, 2, Some(5)),
            ("pendulum", Pendulum, 1, Some(3)),
        ]
    }

    fn stateful(self) -> bool {
        matches!(
            self,
            Func::Lorenz | Func::Rossler | Func::Logistic | Func::Henon | Func::Kuramoto | Func::Pendulum
        )
    }
}

#[derive(Clone, Debug)]
enum Node {
    Num(f64),
    Var(Var),
    Neg(Box<Node>),
    Not(Box<Node>),
    Bin(Op, Box<Node>, Box<Node>),
    Cond(Box<Node>, Box<Node>, Box<Node>),
    /// A function call; `site` indexes the evaluator's state for the
    /// chaotic systems.
    Call(Func, Vec<Node>, usize),
}

/// A compiled expression.
#[derive(Clone, Debug)]
pub struct Expr {
    root: Node,
    sites: usize,
    text: String,
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{{}}}", self.text)
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Op(&'static str),
    LParen,
    RParen,
    Comma,
    Question,
    Colon,
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).map_or(false, |d| d.is_ascii_digit())) {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    i = j;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let s: String = chars[start..i].iter().collect();
            let v: f64 = s.parse().map_err(|_| format!("bad number '{}'", s))?;
            toks.push(Tok::Num(v));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            toks.push(Tok::Ident(chars[start..i].iter().collect()));
            continue;
        }
        let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
        let op2 = ["<=", ">=", "==", "!=", "&&", "||"].into_iter().find(|o| *o == two);
        if let Some(o) = op2 {
            toks.push(Tok::Op(o));
            i += 2;
            continue;
        }
        let t = match c {
            '+' => Tok::Op("+"),
            '-' => Tok::Op("-"),
            '*' => Tok::Op("*"),
            '/' => Tok::Op("/"),
            '%' => Tok::Op("%"),
            '^' => Tok::Op("^"),
            '<' => Tok::Op("<"),
            '>' => Tok::Op(">"),
            '!' => Tok::Op("!"),
            '(' => Tok::LParen,
            ')' => Tok::RParen,
            ',' => Tok::Comma,
            '?' => Tok::Question,
            ':' => Tok::Colon,
            _ => return Err(format!("unexpected '{}'", c)),
        };
        toks.push(t);
        i += 1;
    }
    Ok(toks)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    sites: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn expect(&mut self, want: Tok, what: &str) -> Result<(), String> {
        match self.next() {
            Some(t) if t == want => Ok(()),
            Some(t) => Err(format!("expected {} but found {}", what, show(&t))),
            None => Err(format!("expected {} at the end", what)),
        }
    }

    // ternary, right to left: c ? a : b
    fn expr(&mut self) -> Result<Node, String> {
        let c = self.binary(0)?;
        if self.peek() == Some(&Tok::Question) {
            self.next();
            let a = self.expr()?;
            self.expect(Tok::Colon, "':' of a ? :")?;
            let b = self.expr()?;
            return Ok(Node::Cond(Box::new(c), Box::new(a), Box::new(b)));
        }
        Ok(c)
    }

    fn binary(&mut self, min_prec: u8) -> Result<Node, String> {
        let mut lhs = self.unary()?;
        loop {
            let (op, prec) = match self.peek() {
                Some(Tok::Op(o)) => match *o {
                    "||" => (Op::Or, 1),
                    "&&" => (Op::And, 2),
                    "==" => (Op::Eq, 3),
                    "!=" => (Op::Ne, 3),
                    "<" => (Op::Lt, 4),
                    "<=" => (Op::Le, 4),
                    ">" => (Op::Gt, 4),
                    ">=" => (Op::Ge, 4),
                    "+" => (Op::Add, 5),
                    "-" => (Op::Sub, 5),
                    "*" => (Op::Mul, 6),
                    "/" => (Op::Div, 6),
                    "%" => (Op::Mod, 6),
                    _ => break,
                },
                _ => break,
            };
            if prec < min_prec {
                break;
            }
            self.next();
            let rhs = self.binary(prec + 1)?;
            lhs = Node::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Node, String> {
        match self.peek() {
            Some(Tok::Op("-")) => {
                self.next();
                Ok(Node::Neg(Box::new(self.unary()?)))
            }
            Some(Tok::Op("+")) => {
                self.next();
                self.unary()
            }
            Some(Tok::Op("!")) => {
                self.next();
                Ok(Node::Not(Box::new(self.unary()?)))
            }
            _ => self.power(),
        }
    }

    // ^ binds tighter than unary minus (-x^2 is -(x^2)) and runs right to
    // left (2^3^2 is 2^9)
    fn power(&mut self) -> Result<Node, String> {
        let base = self.atom()?;
        if self.peek() == Some(&Tok::Op("^")) {
            self.next();
            let exp = self.unary()?;
            return Ok(Node::Bin(Op::Pow, Box::new(base), Box::new(exp)));
        }
        Ok(base)
    }

    fn atom(&mut self) -> Result<Node, String> {
        match self.next() {
            Some(Tok::Num(v)) => Ok(Node::Num(v)),
            Some(Tok::LParen) => {
                let e = self.expr()?;
                self.expect(Tok::RParen, "')'")?;
                Ok(e)
            }
            Some(Tok::Ident(name)) => {
                if self.peek() == Some(&Tok::LParen) {
                    self.next();
                    let &(_, func, lo, hi) = Func::table()
                        .iter()
                        .find(|(n, ..)| *n == name)
                        .ok_or_else(|| format!("unknown function '{}'", name))?;
                    let mut args = Vec::new();
                    if self.peek() != Some(&Tok::RParen) {
                        loop {
                            args.push(self.expr()?);
                            if self.peek() == Some(&Tok::Comma) {
                                self.next();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(Tok::RParen, "')' closing the call")?;
                    if args.len() < lo || hi.map_or(false, |h| args.len() > h) {
                        let want = match hi {
                            Some(h) if h == lo => format!("{}", lo),
                            Some(h) => format!("{} to {}", lo, h),
                            None => format!("at least {}", lo),
                        };
                        return Err(format!("{}() takes {} arguments, got {}", name, want, args.len()));
                    }
                    let site = if func.stateful() {
                        self.sites += 1;
                        self.sites - 1
                    } else {
                        usize::MAX
                    };
                    return Ok(Node::Call(func, args, site));
                }
                match name.as_str() {
                    "t" => Ok(Node::Var(Var::T)),
                    "b" => Ok(Node::Var(Var::B)),
                    "s" => Ok(Node::Var(Var::S)),
                    "p" => Ok(Node::Var(Var::P)),
                    "v" => Ok(Node::Var(Var::V)),
                    "pi" => Ok(Node::Num(std::f64::consts::PI)),
                    "tau" => Ok(Node::Num(std::f64::consts::TAU)),
                    "e" => Ok(Node::Num(std::f64::consts::E)),
                    _ => Err(format!(
                        "unknown name '{}' (the clock is t, b, s, p and v; functions take parentheses)",
                        name
                    )),
                }
            }
            Some(t) => Err(format!("unexpected {}", show(&t))),
            None => Err("the expression ends too early".into()),
        }
    }
}

fn show(t: &Tok) -> String {
    match t {
        Tok::Num(v) => format!("{}", v),
        Tok::Ident(s) => format!("'{}'", s),
        Tok::Op(o) => format!("'{}'", o),
        Tok::LParen => "'('".into(),
        Tok::RParen => "')'".into(),
        Tok::Comma => "','".into(),
        Tok::Question => "'?'".into(),
        Tok::Colon => "':'".into(),
    }
}

impl Expr {
    pub fn parse(src: &str) -> Result<Expr, String> {
        let toks = lex(src)?;
        if toks.is_empty() {
            return Err("empty expression".into());
        }
        let mut p = Parser {
            toks,
            pos: 0,
            sites: 0,
        };
        let root = p.expr()?;
        if let Some(t) = p.peek() {
            return Err(format!("unexpected {} after the expression", show(t)));
        }
        Ok(Expr {
            root,
            sites: p.sites,
            text: src.trim().to_string(),
        })
    }

    /// Whether the expression reads a variable: `t` needs the tempo map,
    /// `v` a lane value before it.
    fn reads(&self, var: Var) -> bool {
        fn walk(n: &Node, var: Var) -> bool {
            match n {
                Node::Var(v) => *v == var,
                Node::Num(_) => false,
                Node::Neg(a) | Node::Not(a) => walk(a, var),
                Node::Bin(_, a, b) => walk(a, var) || walk(b, var),
                Node::Cond(c, a, b) => walk(c, var) || walk(a, var) || walk(b, var),
                Node::Call(_, args, _) => args.iter().any(|a| walk(a, var)),
            }
        }
        walk(&self.root, var)
    }

    pub fn reads_time(&self) -> bool {
        self.reads(Var::T)
    }

    pub fn reads_start_value(&self) -> bool {
        self.reads(Var::V)
    }

    /// A fresh evaluator: each segment a token starts gets its own.
    pub fn eval(&self) -> Eval<'_> {
        Eval {
            expr: self,
            state: vec![Site::Fresh; self.sites],
        }
    }
}

/// Integration step of the flows, in their own time units.
const FLOW_H: f64 = 0.002;
/// The double pendulum swings fast: a finer step.
const PENDULUM_H: f64 = 0.0005;
/// The most steps one call site may take, all told (a guard against
/// `lorenz(t * 1e9)`); about a second of work.
const MAX_STEPS: u64 = 40_000_000;
const MAX_OSCILLATORS: usize = 64;

#[derive(Clone, Debug)]
enum Site {
    Fresh,
    /// A flow: the time reached, the state there, steps taken.
    Flow { tau: f64, x: Vec<f64>, steps: u64 },
    /// A map: iterations done and the state.
    Map { n: u64, x: [f64; 2] },
}

/// Evaluates one expression along increasing time, carrying the chaotic
/// systems' state from one control step to the next.
pub struct Eval<'a> {
    expr: &'a Expr,
    state: Vec<Site>,
}

impl Eval<'_> {
    pub fn value(&mut self, ctx: &Ctx) -> Result<f64, String> {
        let root = &self.expr.root;
        eval_node(root, ctx, &mut self.state)
    }
}

fn truth(x: f64) -> bool {
    x != 0.0 && !x.is_nan()
}

fn eval_node(n: &Node, ctx: &Ctx, st: &mut [Site]) -> Result<f64, String> {
    Ok(match n {
        Node::Num(v) => *v,
        Node::Var(v) => match v {
            Var::T => ctx.t,
            Var::B => ctx.b,
            Var::S => ctx.s,
            Var::P => ctx.p,
            Var::V => ctx.v,
        },
        Node::Neg(a) => -eval_node(a, ctx, st)?,
        Node::Not(a) => {
            if truth(eval_node(a, ctx, st)?) {
                0.0
            } else {
                1.0
            }
        }
        Node::Bin(op, a, b) => {
            let x = eval_node(a, ctx, st)?;
            // && and || look at the right side only when they must
            match op {
                Op::And if !truth(x) => return Ok(0.0),
                Op::Or if truth(x) => return Ok(1.0),
                _ => {}
            }
            let y = eval_node(b, ctx, st)?;
            let flag = |c: bool| if c { 1.0 } else { 0.0 };
            match op {
                Op::Add => x + y,
                Op::Sub => x - y,
                Op::Mul => x * y,
                Op::Div => x / y,
                Op::Mod => x.rem_euclid(y),
                Op::Pow => x.powf(y),
                Op::Lt => flag(x < y),
                Op::Le => flag(x <= y),
                Op::Gt => flag(x > y),
                Op::Ge => flag(x >= y),
                Op::Eq => flag(x == y),
                Op::Ne => flag(x != y),
                Op::And | Op::Or => flag(truth(y)),
            }
        }
        Node::Cond(c, a, b) => {
            if truth(eval_node(c, ctx, st)?) {
                eval_node(a, ctx, st)?
            } else {
                eval_node(b, ctx, st)?
            }
        }
        Node::Call(f, args, site) => {
            let mut v = [0.0f64; 8];
            let mut list = Vec::new();
            let many = matches!(f, Func::Min | Func::Max | Func::Seq);
            for (i, a) in args.iter().enumerate() {
                let x = eval_node(a, ctx, st)?;
                if many {
                    list.push(x);
                } else {
                    v[i] = x;
                }
            }
            let n = args.len();
            let arg = |i: usize, default: f64| if i < n { v[i] } else { default };
            call(*f, &v, &list, &arg, if *site == usize::MAX { None } else { Some(&mut st[*site]) })?
        }
    })
}

fn call(
    f: Func,
    v: &[f64; 8],
    list: &[f64],
    arg: &dyn Fn(usize, f64) -> f64,
    site: Option<&mut Site>,
) -> Result<f64, String> {
    use std::f64::consts::TAU;
    let x = v[0];
    Ok(match f {
        Func::Sin => x.sin(),
        Func::Cos => x.cos(),
        Func::Tan => x.tan(),
        Func::Asin => x.asin(),
        Func::Acos => x.acos(),
        Func::Atan => x.atan(),
        Func::Atan2 => x.atan2(v[1]),
        Func::Sinh => x.sinh(),
        Func::Cosh => x.cosh(),
        Func::Tanh => x.tanh(),
        Func::Exp => x.exp(),
        Func::Exp2 => x.exp2(),
        Func::Log => x.ln(),
        Func::Log2 => x.log2(),
        Func::Log10 => x.log10(),
        Func::Sqrt => x.sqrt(),
        Func::Abs => x.abs(),
        Func::Sign => {
            if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
        Func::Floor => x.floor(),
        Func::Ceil => x.ceil(),
        Func::Round => x.round(),
        Func::Fract => x - x.floor(),
        Func::Min => list.iter().copied().fold(f64::INFINITY, f64::min),
        Func::Max => list.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        Func::Clamp => x.max(v[1]).min(v[2]),
        Func::Mix => x + (v[1] - x) * v[2],
        Func::Pow => x.powf(v[1]),
        Func::Smoothstep => {
            let (e0, e1, y) = (x, v[1], v[2]);
            let u = ((y - e0) / (e1 - e0)).clamp(0.0, 1.0);
            u * u * (3.0 - 2.0 * u)
        }
        Func::Snap => {
            if v[1] == 0.0 {
                x
            } else {
                (x / v[1]).round() * v[1]
            }
        }
        Func::Mtof => 440.0 * ((x - 69.0) / 12.0).exp2(),
        Func::Db => 10f64.powf(x / 20.0),
        Func::Sine => (TAU * x).sin(),
        Func::Tri => {
            let ph = x - x.floor();
            1.0 - 4.0 * (ph - 0.5).abs()
        }
        Func::Saw => 2.0 * (x - x.floor()) - 1.0,
        Func::Square => {
            if x - x.floor() < arg(1, 0.5) {
                1.0
            } else {
                -1.0
            }
        }
        Func::Step => {
            if x >= 0.0 {
                1.0
            } else {
                0.0
            }
        }
        Func::Seq => {
            let items = &list[1..];
            let i = list[0].floor();
            if !i.is_finite() {
                return Err("seq() index is not a number".into());
            }
            items[(i as i64).rem_euclid(items.len() as i64) as usize]
        }
        Func::Noise => gradient_noise(x, arg(1, 0.0)),
        Func::Rand => hash01(x.floor(), arg(1, 0.0)),
        Func::Lorenz => {
            let axis = axis_of(arg(1, 0.0), 3, "lorenz")?;
            let (rho, sigma, beta) = (arg(2, 28.0), arg(3, 10.0), arg(4, 8.0 / 3.0));
            let s = flow(site.unwrap(), x, FLOW_H, &[1.0, 1.0, 1.0], "lorenz", |s, d| {
                d[0] = sigma * (s[1] - s[0]);
                d[1] = s[0] * (rho - s[2]) - s[1];
                d[2] = s[0] * s[1] - beta * s[2];
            })?;
            match axis {
                0 => s[0] / 20.0,
                1 => s[1] / 27.0,
                _ => (s[2] - 25.0) / 25.0,
            }
        }
        Func::Rossler => {
            let axis = axis_of(arg(1, 0.0), 3, "rossler")?;
            let (a, b, c) = (arg(2, 0.2), arg(3, 0.2), arg(4, 5.7));
            let s = flow(site.unwrap(), x, FLOW_H, &[1.0, 1.0, 0.0], "rossler", |s, d| {
                d[0] = -s[1] - s[2];
                d[1] = s[0] + a * s[1];
                d[2] = b + s[2] * (s[0] - c);
            })?;
            s[axis] / 12.0
        }
        Func::Logistic => {
            let r = v[1].clamp(0.0, 4.0);
            let x0 = arg(2, 0.4).clamp(0.0, 1.0);
            let st = map(site.unwrap(), x, [x0, 0.0], "logistic", |s| {
                [r * s[0] * (1.0 - s[0]), 0.0]
            })?;
            st[0]
        }
        Func::Henon => {
            let axis = axis_of(arg(1, 0.0), 2, "henon")?;
            let (a, b) = (arg(2, 1.4), arg(3, 0.3));
            let st = map(site.unwrap(), x, [0.0, 0.0], "henon", |s| {
                let nx = 1.0 - a * s[0] * s[0] + s[1];
                let ny = b * s[0];
                // an orbit thrown off the attractor starts over
                if nx.abs() > 1e6 || !nx.is_finite() {
                    [0.0, 0.0]
                } else {
                    [nx, ny]
                }
            })?;
            if axis == 0 {
                st[0] / 1.3
            } else {
                st[1] / 0.4
            }
        }
        Func::Kuramoto => {
            let k = v[1];
            let count = arg(3, 8.0).round();
            if !(1.0..=MAX_OSCILLATORS as f64).contains(&count) {
                return Err(format!("kuramoto() takes 1 to {} oscillators", MAX_OSCILLATORS));
            }
            let count = count as usize;
            let which = arg(2, 0.0).round();
            if which < -1.0 || which > count as f64 {
                return Err(format!("kuramoto() voice {} of {}", which, count));
            }
            let spread = arg(4, 0.5);
            let init: Vec<f64> = (0..count).map(|i| TAU * (i as f64 * 0.618_033_988_75).fract()).collect();
            let s = flow(site.unwrap(), x, FLOW_H, &init, "kuramoto", |s, d| {
                let nf = s.len() as f64;
                let (sum_c, sum_s) = s.iter().fold((0.0, 0.0), |a, th| (a.0 + th.cos(), a.1 + th.sin()));
                for (i, th) in s.iter().enumerate() {
                    let w = if s.len() > 1 {
                        1.0 + spread * (2.0 * i as f64 / (nf - 1.0) - 1.0)
                    } else {
                        1.0
                    };
                    // (k/n) sum_j sin(th_j - th_i) = (k/n)(S cos th_i - C sin th_i)
                    d[i] = TAU * w + k / nf * (sum_s * th.cos() - sum_c * th.sin());
                }
            })?;
            let nf = count as f64;
            match which as i64 {
                -1 => {
                    let (c, sn) = s.iter().fold((0.0, 0.0), |a, th| (a.0 + th.cos(), a.1 + th.sin()));
                    (c * c + sn * sn).sqrt() / nf
                }
                0 => s.iter().map(|th| th.cos()).sum::<f64>() / nf,
                i => s[(i - 1) as usize].cos(),
            }
        }
        Func::Pendulum => {
            let axis = axis_of(arg(1, 0.0), 3, "pendulum")?;
            let damping = arg(2, 0.0).max(0.0);
            // equal arms and bobs; g/l set so a small swing takes about
            // one time unit
            let g = TAU * TAU;
            let s = flow(site.unwrap(), x, PENDULUM_H, &[2.0, 2.5, 0.0, 0.0], "pendulum", |s, d| {
                let (t1, t2, w1, w2) = (s[0], s[1], s[2], s[3]);
                let dl = t1 - t2;
                let den = 3.0 - (2.0 * dl).cos();
                d[0] = w1;
                d[1] = w2;
                d[2] = (-3.0 * g * t1.sin() - g * (t1 - 2.0 * t2).sin()
                    - 2.0 * dl.sin() * (w2 * w2 + w1 * w1 * dl.cos()))
                    / den
                    - damping * w1;
                d[3] = (2.0 * dl.sin() * (2.0 * w1 * w1 + 2.0 * g * t1.cos() + w2 * w2 * dl.cos())) / den
                    - damping * w2;
            })?;
            match axis {
                0 => s[0].sin(),
                1 => s[1].sin(),
                _ => 0.5 * (s[0].sin() + s[1].sin()),
            }
        }
    })
}

fn axis_of(a: f64, n: usize, name: &str) -> Result<usize, String> {
    let i = a.round();
    if i < 0.0 || i >= n as f64 || !i.is_finite() {
        return Err(format!("{}() axis {} (0 to {})", name, a, n - 1));
    }
    Ok(i as usize)
}

/// Integrate a flow from time 0 to `time` with fixed RK4 steps, carrying
/// the last committed step in `site`; the remainder is a partial step
/// from there, so the value is smooth in `time`.
fn flow(
    site: &mut Site,
    time: f64,
    h: f64,
    init: &[f64],
    name: &str,
    f: impl Fn(&[f64], &mut [f64]),
) -> Result<Vec<f64>, String> {
    if !time.is_finite() {
        return Err(format!("{}() time is not a number", name));
    }
    let time = time.max(0.0);
    let restart = match site {
        Site::Flow { tau, x, .. } => *tau > time || x.len() != init.len(),
        _ => true,
    };
    if restart {
        *site = Site::Flow {
            tau: 0.0,
            x: init.to_vec(),
            steps: 0,
        };
    }
    let Site::Flow { tau, x, steps } = site else {
        unreachable!()
    };
    let dim = x.len();
    let mut k = vec![vec![0.0; dim]; 4];
    let mut tmp = vec![0.0; dim];
    let rk4 = |x: &mut Vec<f64>, dt: f64, k: &mut Vec<Vec<f64>>, tmp: &mut Vec<f64>| {
        f(x, &mut k[0]);
        for i in 0..dim {
            tmp[i] = x[i] + 0.5 * dt * k[0][i];
        }
        f(tmp, &mut k[1]);
        for i in 0..dim {
            tmp[i] = x[i] + 0.5 * dt * k[1][i];
        }
        f(tmp, &mut k[2]);
        for i in 0..dim {
            tmp[i] = x[i] + dt * k[2][i];
        }
        f(tmp, &mut k[3]);
        for i in 0..dim {
            x[i] += dt / 6.0 * (k[0][i] + 2.0 * k[1][i] + 2.0 * k[2][i] + k[3][i]);
        }
    };
    while *tau + h <= time {
        rk4(x, h, &mut k, &mut tmp);
        *tau += h;
        *steps += 1;
        if *steps > MAX_STEPS {
            return Err(format!("{}() was asked to run too far (time {})", name, time));
        }
    }
    let mut out = x.clone();
    let rest = time - *tau;
    if rest > 0.0 {
        rk4(&mut out, rest, &mut k, &mut tmp);
    }
    if out.iter().any(|v| !v.is_finite()) {
        return Err(format!("{}() diverged at time {}", name, time));
    }
    Ok(out)
}

/// Iterate a map floor(count) times from `init`, carrying the count done.
fn map(
    site: &mut Site,
    count: f64,
    init: [f64; 2],
    name: &str,
    f: impl Fn([f64; 2]) -> [f64; 2],
) -> Result<[f64; 2], String> {
    if !count.is_finite() {
        return Err(format!("{}() count is not a number", name));
    }
    let target = count.max(0.0).floor() as u64;
    if target > MAX_STEPS {
        return Err(format!("{}() was asked for {} iterations", name, target));
    }
    let restart = match site {
        Site::Map { n, .. } => *n > target,
        _ => true,
    };
    if restart {
        *site = Site::Map { n: 0, x: init };
    }
    let Site::Map { n, x } = site else {
        unreachable!()
    };
    while *n < target {
        *x = f(*x);
        *n += 1;
    }
    Ok(*x)
}

fn hash_u64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A uniform 0..1 for an integer cell and a seed.
fn hash01(cell: f64, seed: f64) -> f64 {
    let c = cell as i64 as u64;
    let s = (seed.round() as i64 as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    (hash_u64(c ^ s) >> 11) as f64 / (1u64 << 53) as f64
}

/// 1-D gradient (Perlin) noise: a random slope at every integer, blended
/// with the quintic fade, scaled to reach about -1..1.
fn gradient_noise(x: f64, seed: f64) -> f64 {
    let i = x.floor();
    let f = x - i;
    let g0 = hash01(i, seed) * 2.0 - 1.0;
    let g1 = hash01(i + 1.0, seed) * 2.0 - 1.0;
    let fade = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let a = g0 * f;
    let b = g1 * (f - 1.0);
    2.0 * (a + (b - a) * fade)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(src: &str, ctx: Ctx) -> f64 {
        Expr::parse(src).unwrap().eval().value(&ctx).unwrap()
    }

    fn b(beat: f64) -> Ctx {
        Ctx { b: beat, t: beat * 0.5, ..Ctx::default() }
    }

    #[test]
    fn arithmetic_follows_the_usual_precedence() {
        assert_eq!(at("1 + 2 * 3", b(0.0)), 7.0);
        assert_eq!(at("(1 + 2) * 3", b(0.0)), 9.0);
        assert_eq!(at("-2^2", b(0.0)), -4.0);
        assert_eq!(at("2^3^2", b(0.0)), 512.0);
        assert_eq!(at("7 % 3 + -7 % 3", b(0.0)), 3.0);
        assert_eq!(at("b > 2 ? 10 : 20", b(3.0)), 10.0);
        assert_eq!(at("b > 2 && b < 4", b(5.0)), 0.0);
        assert_eq!(at("!(b == 1) || 0", b(1.0)), 0.0);
        assert!((at("sin(pi / 2) + cos(0)", b(0.0)) - 2.0).abs() < 1e-12);
        assert!((at("mtof(69) + db(-6.0206)", b(0.0)) - 440.5).abs() < 1e-4);
        assert_eq!(at("max(1, 5, 3) - min(4, 2)", b(0.0)), 3.0);
        assert_eq!(at("clamp(b, 0, 1) + snap(0.37, 0.25)", b(3.0)), 1.25);
    }

    #[test]
    fn clock_variables_reach_the_expression() {
        let ctx = Ctx { t: 1.5, b: 3.0, s: 1.0, p: 0.25, v: 100.0 };
        assert_eq!(at("t + b + s + p + v", ctx), 105.75);
        let e = Expr::parse("v * (1 + 0.1 * sin(t))").unwrap();
        assert!(e.reads_time() && e.reads_start_value());
        assert!(!Expr::parse("b").unwrap().reads_time());
    }

    #[test]
    fn errors_say_what_is_wrong() {
        assert!(Expr::parse("sin(1, 2)").unwrap_err().contains("takes 1"));
        assert!(Expr::parse("wobble(1)").unwrap_err().contains("unknown function"));
        assert!(Expr::parse("x + 1").unwrap_err().contains("unknown name"));
        assert!(Expr::parse("(1 + 2").is_err());
        assert!(Expr::parse("1 2").is_err());
        assert!(Expr::parse("").is_err());
        assert!(Expr::parse("lorenz(t, 3)").unwrap().eval().value(&b(1.0)).is_err());
    }

    #[test]
    fn cycle_shapes_have_period_one_and_span_minus_one_to_one() {
        for (shape, at_quarter) in [("sine", 1.0), ("tri", 0.0), ("saw", -0.5), ("square", 1.0)] {
            let e = Expr::parse(&format!("{}(b)", shape)).unwrap();
            let mut ev = e.eval();
            let v1 = ev.value(&b(0.25)).unwrap();
            let v2 = ev.value(&b(1.25)).unwrap();
            assert!((v1 - at_quarter).abs() < 1e-12, "{shape}(0.25) = {v1}");
            assert!((v1 - v2).abs() < 1e-12, "{shape} repeats each unit");
        }
        assert_eq!(at("step(b - 2)", b(2.0)), 1.0);
        assert_eq!(at("step(b - 2)", b(1.999)), 0.0);
        // seq cycles through its list, one entry per unit of its index
        let vals: Vec<f64> = (0..6).map(|i| at("seq(b, 10, 20, 30)", b(i as f64 + 0.5))).collect();
        assert_eq!(vals, vec![10.0, 20.0, 30.0, 10.0, 20.0, 30.0]);
    }

    #[test]
    fn noise_is_smooth_bounded_and_repeatable() {
        let e = Expr::parse("noise(b * 4)").unwrap();
        let mut ev = e.eval();
        let mut last = ev.value(&b(0.0)).unwrap();
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for i in 1..4000 {
            let v = ev.value(&b(i as f64 / 1000.0)).unwrap();
            // continuous: no jump between neighbouring control steps
            assert!((v - last).abs() < 0.05, "jump at step {i}");
            lo = lo.min(v);
            hi = hi.max(v);
            last = v;
        }
        assert!(lo >= -1.0 && hi <= 1.0 && hi - lo > 0.5, "{lo}..{hi}");
        assert_eq!(at("noise(3.3, 7)", b(0.0)), at("noise(3.3, 7)", b(9.0)));
        assert_ne!(at("noise(3.3, 7)", b(0.0)), at("noise(3.3, 8)", b(0.0)));
        // rand holds one value per cell and differs between cells
        assert_eq!(at("rand(b)", b(2.1)), at("rand(b)", b(2.9)));
        assert_ne!(at("rand(b)", b(2.1)), at("rand(b)", b(3.1)));
    }

    #[test]
    fn logistic_map_doubles_its_period_then_turns_chaotic() {
        // settle 2000 iterations, then count the distinct values visited
        let distinct = |r: f64| {
            let e = Expr::parse(&format!("logistic(b, {r})")).unwrap();
            let mut ev = e.eval();
            let mut seen: Vec<f64> = Vec::new();
            for n in 2000..2064 {
                let x = ev.value(&b(n as f64)).unwrap();
                assert!((0.0..=1.0).contains(&x));
                if !seen.iter().any(|s| (s - x).abs() < 1e-6) {
                    seen.push(x);
                }
            }
            seen.len()
        };
        assert_eq!(distinct(2.8), 1, "a fixed point below r = 3");
        assert_eq!(distinct(3.2), 2, "period 2");
        assert_eq!(distinct(3.5), 4, "period 4");
        assert!(distinct(3.9) > 30, "chaos");
        // the fixed point is 1 - 1/r
        let x = at("logistic(5000, 2.8)", b(0.0));
        assert!((x - (1.0 - 1.0 / 2.8)).abs() < 1e-9);
    }

    #[test]
    fn lorenz_stays_on_its_attractor_and_is_a_pure_function_of_time() {
        let e = Expr::parse("lorenz(t, 0)").unwrap();
        let mut ev = e.eval();
        let mut crossings = 0;
        let mut last = 0.0f64;
        for i in 0..20_000 {
            let ctx = Ctx { t: i as f64 * 0.005, ..Ctx::default() };
            let x = ev.value(&ctx).unwrap();
            assert!(x.abs() < 1.2, "x/20 = {x} off the attractor");
            if i > 0 && x.signum() != last.signum() {
                crossings += 1;
            }
            last = x;
        }
        // the orbit hops between the two lobes, irregularly
        assert!(crossings > 10, "{crossings} lobe changes in 100 time units");
        // asking at time 50 directly gives what stepping there gave
        let direct = at("lorenz(t, 0)", Ctx { t: 50.0, ..Ctx::default() });
        let mut ev = e.eval();
        let mut stepped = 0.0;
        for i in 0..=1000 {
            stepped = ev.value(&Ctx { t: i as f64 * 0.05, ..Ctx::default() }).unwrap();
        }
        assert!((direct - stepped).abs() < 1e-9, "{direct} vs {stepped}");
        // and z sits above the plane on average (the butterfly's wings)
        let z = at("lorenz(t, 2)", Ctx { t: 30.0, ..Ctx::default() });
        assert!(z > -1.0 && z < 1.0);
    }

    #[test]
    fn kuramoto_swarm_locks_above_critical_coupling_and_drifts_below() {
        // n = 16 with natural frequencies spread +-0.5 cycles a unit
        // (+-pi rad a unit): uniform spread locks near k = 4 * spread * 2pi / pi
        let coherence = |k: f64| {
            let e = Expr::parse(&format!("kuramoto(t, {k}, -1, 16, 0.5)")).unwrap();
            let mut ev = e.eval();
            let mut acc = 0.0;
            let mut n = 0;
            for i in 0..4000 {
                let r = ev.value(&Ctx { t: i as f64 * 0.01, ..Ctx::default() }).unwrap();
                if i >= 2000 {
                    acc += r;
                    n += 1;
                }
            }
            acc / n as f64
        };
        let (weak, strong) = (coherence(0.5), coherence(12.0));
        assert!(weak < 0.45, "uncoupled swarm should stay apart: {weak}");
        assert!(strong > 0.95, "strongly coupled swarm should lock: {strong}");
    }

    #[test]
    fn double_pendulum_swings_and_damping_brings_it_to_rest() {
        // undamped it keeps swinging; damped, it comes to rest hanging down
        let e = Expr::parse("pendulum(t, 0)").unwrap();
        let mut ev = e.eval();
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for i in 1000..2000 {
            let v = ev.value(&Ctx { t: i as f64 * 0.01, ..Ctx::default() }).unwrap();
            lo = lo.min(v);
            hi = hi.max(v);
        }
        assert!(hi - lo > 1.0, "undamped, still swinging wide at t = 10..20: {lo}..{hi}");
        let rest = at("pendulum(t, 1, 3)", Ctx { t: 40.0, ..Ctx::default() });
        assert!(rest.abs() < 0.02, "damped lower arm still swinging: {rest}");
    }
}
