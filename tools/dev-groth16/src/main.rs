//! Throwaway Groth16 setup and prover for a circom R1CS, for development only.
//!
//!   dev-groth16 setup <circuit.r1cs> <proving-key.bin> <verification_key.json>
//!   dev-groth16 prove <circuit.r1cs> <proving-key.bin> <witness.wtns> <proof.json> <public.json>
//!
//! The setup draws its toxic waste from the OS and keeps nothing, but it is a single-party setup with no
//! Powers of Tau: whoever ran it could have forged proofs. The ceremony (`circuits/scripts/setup.sh`
//! with snarkjs, ticket 12) replaces it. Outputs use snarkjs' JSON layout so `zolar_claim::private_claim`
//! and `groth16-solana`'s `circom-vk` generator read them unchanged.

use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Seek, SeekFrom},
    path::Path,
};

use ark_bn254::{Bn254, Fq, Fq2, Fr, G1Affine, G2Affine};
use ark_ff::PrimeField;
use ark_groth16::{Groth16, ProvingKey};
use ark_relations::r1cs::{
    ConstraintSynthesizer, ConstraintSystemRef, LinearCombination, SynthesisError, Variable,
};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use ark_snark::SNARK;
use rand::rngs::OsRng;
use serde_json::{json, Value};

type Lc = Vec<(u32, Fr)>;

/// A circom R1CS (iden3 binary format) and, for proving, its witness.
struct Circuit {
    n_wires: usize,
    n_public: usize,
    constraints: Vec<[Lc; 3]>,
    witness: Option<Vec<Fr>>,
}

fn read_u32(r: &mut impl Read) -> u32 {
    let mut b = [0u8; 4];
    r.read_exact(&mut b).unwrap();
    u32::from_le_bytes(b)
}

fn read_u64(r: &mut impl Read) -> u64 {
    let mut b = [0u8; 8];
    r.read_exact(&mut b).unwrap();
    u64::from_le_bytes(b)
}

fn read_fr(r: &mut impl Read, n8: usize) -> Fr {
    let mut b = vec![0u8; n8];
    r.read_exact(&mut b).unwrap();
    Fr::from_le_bytes_mod_order(&b)
}

fn read_r1cs(path: &Path) -> Circuit {
    let mut r = BufReader::with_capacity(1 << 20, File::open(path).expect("r1cs"));
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).unwrap();
    assert_eq!(&magic, b"r1cs");
    assert_eq!(read_u32(&mut r), 1, "r1cs version");
    let sections = read_u32(&mut r);
    // circom writes the constraints before the header, so find the sections first.
    let mut at = std::collections::HashMap::new();
    for _ in 0..sections {
        let id = read_u32(&mut r);
        let size = read_u64(&mut r);
        at.insert(id, r.stream_position().unwrap());
        r.seek(SeekFrom::Current(size as i64)).unwrap();
    }
    r.seek(SeekFrom::Start(at[&1])).unwrap();
    let n8 = read_u32(&mut r) as usize;
    assert_eq!(n8, 32, "field size");
    let mut prime = vec![0u8; n8];
    r.read_exact(&mut prime).unwrap();
    let n_wires = read_u32(&mut r) as usize;
    let n_pub_out = read_u32(&mut r) as usize;
    let n_pub_in = read_u32(&mut r) as usize;
    let _private = read_u32(&mut r);
    let _labels = read_u64(&mut r);
    let n_constraints = read_u32(&mut r) as usize;
    r.seek(SeekFrom::Start(at[&2])).unwrap();
    let mut constraints = Vec::with_capacity(n_constraints);
    for _ in 0..n_constraints {
        let mut c: [Lc; 3] = Default::default();
        for lc in &mut c {
            let terms = read_u32(&mut r) as usize;
            lc.reserve_exact(terms);
            for _ in 0..terms {
                let wire = read_u32(&mut r);
                lc.push((wire, read_fr(&mut r, n8)));
            }
        }
        constraints.push(c);
    }
    Circuit {
        n_wires,
        n_public: n_pub_out + n_pub_in,
        constraints,
        witness: None,
    }
}

fn read_wtns(path: &Path) -> Vec<Fr> {
    let mut r = BufReader::with_capacity(1 << 20, File::open(path).expect("wtns"));
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).unwrap();
    assert_eq!(&magic, b"wtns");
    read_u32(&mut r);
    let sections = read_u32(&mut r);
    let (mut n8, mut count) = (0usize, 0usize);
    let mut values = Vec::new();
    for _ in 0..sections {
        let id = read_u32(&mut r);
        let size = read_u64(&mut r);
        match id {
            1 => {
                n8 = read_u32(&mut r) as usize;
                let mut prime = vec![0u8; n8];
                r.read_exact(&mut prime).unwrap();
                count = read_u32(&mut r) as usize;
            }
            2 => {
                values.reserve(count);
                for _ in 0..count {
                    values.push(read_fr(&mut r, n8));
                }
            }
            _ => {
                std::io::copy(&mut (&mut r).take(size), &mut std::io::sink()).unwrap();
            }
        }
    }
    values
}

impl ConstraintSynthesizer<Fr> for Circuit {
    fn generate_constraints(self, cs: ConstraintSystemRef<Fr>) -> Result<(), SynthesisError> {
        let value = |wire: usize| -> Result<Fr, SynthesisError> {
            self.witness
                .as_ref()
                .map(|w| w[wire])
                .ok_or(SynthesisError::AssignmentMissing)
        };
        let mut vars = Vec::with_capacity(self.n_wires);
        vars.push(Variable::One);
        for wire in 1..self.n_wires {
            vars.push(if wire <= self.n_public {
                cs.new_input_variable(|| value(wire))?
            } else {
                cs.new_witness_variable(|| value(wire))?
            });
        }
        let lc = |terms: &Lc| {
            let mut out = LinearCombination::zero();
            for (wire, coeff) in terms {
                out += (*coeff, vars[*wire as usize]);
            }
            out
        };
        for [a, b, c] in &self.constraints {
            cs.enforce_constraint(lc(a), lc(b), lc(c))?;
        }
        Ok(())
    }
}

fn fq(x: &Fq) -> Value {
    json!(x.into_bigint().to_string())
}

fn g1(p: &G1Affine) -> Value {
    json!([fq(&p.x), fq(&p.y), "1"])
}

fn fq2(x: &Fq2) -> Value {
    json!([fq(&x.c0), fq(&x.c1)])
}

fn g2(p: &G2Affine) -> Value {
    json!([fq2(&p.x), fq2(&p.y), ["1", "0"]])
}

fn write_json(path: &str, value: &Value) {
    serde_json::to_writer(BufWriter::new(File::create(path).unwrap()), value).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["setup", r1cs, pk_out, vk_out] => {
            let circuit = read_r1cs(Path::new(r1cs));
            let n_public = circuit.n_public;
            eprintln!(
                "{} constraints, {} wires",
                circuit.constraints.len(),
                circuit.n_wires
            );
            let (pk, vk) = Groth16::<Bn254>::circuit_specific_setup(circuit, &mut OsRng).unwrap();
            pk.serialize_uncompressed(BufWriter::new(File::create(pk_out).unwrap()))
                .unwrap();
            write_json(
                vk_out,
                &json!({
                    "protocol": "groth16",
                    "curve": "bn128",
                    "nPublic": n_public,
                    "vk_alpha_1": g1(&vk.alpha_g1),
                    "vk_beta_2": g2(&vk.beta_g2),
                    "vk_gamma_2": g2(&vk.gamma_g2),
                    "vk_delta_2": g2(&vk.delta_g2),
                    "IC": vk.gamma_abc_g1.iter().map(g1).collect::<Vec<_>>(),
                }),
            );
        }
        ["prove", r1cs, pk_path, wtns, proof_out, public_out] => {
            let mut circuit = read_r1cs(Path::new(r1cs));
            let n_public = circuit.n_public;
            let witness = read_wtns(Path::new(wtns));
            assert_eq!(
                witness.len(),
                circuit.n_wires,
                "witness does not fit the R1CS"
            );
            let public: Vec<Value> = witness[1..=n_public]
                .iter()
                .map(|f| json!(f.into_bigint().to_string()))
                .collect();
            circuit.witness = Some(witness);
            let pk = ProvingKey::<Bn254>::deserialize_uncompressed_unchecked(BufReader::new(
                File::open(pk_path).unwrap(),
            ))
            .unwrap();
            let proof = Groth16::<Bn254>::prove(&pk, circuit, &mut OsRng).unwrap();
            write_json(
                proof_out,
                &json!({
                    "pi_a": g1(&proof.a),
                    "pi_b": g2(&proof.b),
                    "pi_c": g1(&proof.c),
                    "protocol": "groth16",
                    "curve": "bn128",
                }),
            );
            write_json(public_out, &Value::Array(public));
        }
        _ => {
            eprintln!("usage: dev-groth16 setup <r1cs> <pk-out> <vk-out.json>\n       dev-groth16 prove <r1cs> <pk> <wtns> <proof.json> <public.json>");
            std::process::exit(2);
        }
    }
}
