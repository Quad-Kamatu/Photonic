#[test]
#[ignore]
fn embeds_and_ranks() {
    let e = photonic_embed::Embedder::new().expect("load model");
    let corpus = vec![
        "Draw ellipses and circles".to_string(),
        "Add text to the canvas".to_string(),
        "Undo the last action".to_string(),
    ];
    let vecs = e.embed(&corpus).expect("embed corpus");
    assert_eq!(vecs.len(), 3);
    assert_eq!(vecs[0].len(), 384, "MiniLM is 384-dim");
    let q = e.embed(&["round shape".to_string()]).expect("embed query");
    let sims: Vec<f32> = vecs.iter().map(|v| photonic_embed::cosine(&q[0], v)).collect();
    println!("sims (ellipse/text/undo) = {sims:?}");
    assert!(sims[0] > sims[1] && sims[0] > sims[2], "ellipse should win: {sims:?}");
}
