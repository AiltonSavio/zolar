use groth16_solana::groth16::Groth16Verifyingkey;
pub const VERIFYINGKEY: Groth16Verifyingkey = Groth16Verifyingkey {
    nr_pubinputs: 4,
    vk_alpha_g1: [0; 64],
    vk_beta_g2: [0; 128],
    vk_gamma_g2: [0; 128],
    vk_delta_g2: [0; 128],
    vk_ic: &[[0; 64]; 5],
    vk_commitment: None,
};
