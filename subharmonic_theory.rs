fn test_veto(relative_amp: f32) -> (f32, f32, f32) {
    let original = 50.0 * (2.5 * relative_amp).exp();
    let current = if relative_amp > 0.03 {
        5.0 * ((2.5 * relative_amp).exp() - 1.0)
    } else {
        0.0
    };
    let quadratic = 30.0 * relative_amp.powi(2);
    (original, current, quadratic)
}

fn main() {
    println!("{:>10} | {:>10} | {:>10} | {:>10}", "Rel Amp", "Original", "Current", "Quadratic");
    println!("{:-<49}", "-");
    for x in [0.0, 0.01, 0.02, 0.03, 0.04, 0.05, 0.08, 0.1, 0.15, 0.2, 0.5, 1.0].iter() {
        let (o, c, q) = test_veto(*x);
        println!("{:>10.3} | {:>10.3} | {:>10.3} | {:>10.3}", x, o, c, q);
    }
}
