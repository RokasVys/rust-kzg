use crate::data_types::{fp::*, fr::*, g2::*};
use crate::fk20_fft::FFTSettings as mFFTSettings;
use crate::kzg_settings::KZGSettings as mKZGSettings;
use crate::utilities::reverse_bit_order;

use kzg::cfg_into_iter;
use kzg::eip_4844::{
    blst_p1, load_trusted_setup_string, Blob, Bytes32, Bytes48, CKZGSettings,
    KZGCommitment, KZGProof, BYTES_PER_FIELD_ELEMENT, BYTES_PER_G1, BYTES_PER_G2, C_KZG_RET,
    C_KZG_RET_BADARGS, C_KZG_RET_OK,
};
use std::slice::{from_raw_parts, from_raw_parts_mut};

use crate::data_types::g1::G1;
use kzg::G1 as CommonG1;

#[cfg(feature = "parallel")]
use rayon::prelude::*;

macro_rules! handle_ckzg_badargs {
    ($x: expr) => {
        match $x {
            Ok(value) => value,
            Err(_) => return C_KZG_RET_BADARGS,
        }
    };
}

unsafe fn cg1_to_g1(t: *const blst_p1) -> G1 {
    G1 {
        x: Fp { d: (*t).x.l },
        y: Fp { d: (*t).y.l },
        z: Fp { d: (*t).z.l },
    }
}

unsafe fn ks_to_cks(t: &mut mKZGSettings, out: *mut CKZGSettings) {
    assert_eq!(t.curve.g1_points.len(), t.fft_settings.max_width);
    (*out).g1_values = t.curve.g1_points.as_mut_ptr() as _;
    (*out).g2_values = t.curve.g2_points.as_mut_ptr() as _;
    (*out).max_width = t.fft_settings.max_width as _;
    (*out).roots_of_unity = t.fft_settings.expanded_roots_of_unity.as_mut_ptr() as _;
}

unsafe fn cks_to_ks(t: *const CKZGSettings) -> mKZGSettings {
    crate::fk20_fft::init_globals();
    let mw = (*t).max_width as usize;
    let roots_of_unity = Vec::from_raw_parts((*t).roots_of_unity as _, mw + 1, mw + 1);
    let mut expanded_roots_of_unity = roots_of_unity.clone();
    reverse_bit_order(&mut expanded_roots_of_unity);
    expanded_roots_of_unity.push(Fr::one());
    let mut reverse_roots_of_unity = expanded_roots_of_unity.clone();
    reverse_roots_of_unity.reverse();

    let mut first_root = expanded_roots_of_unity[1];
    let first_root_arr = [first_root; 1];
    first_root = first_root_arr[0];
    mKZGSettings {
        curve: crate::kzg10::Curve {
            g1_gen: G1::gen(),
            g2_gen: G2::gen(),
            g1_points: Vec::from_raw_parts((*t).g1_values as _, mw, mw),
            g2_points: Vec::from_raw_parts((*t).g2_values as _, 65, 65),
        },
        fft_settings: mFFTSettings {
            max_width: mw,
            root_of_unity: first_root,
            expanded_roots_of_unity,
            reverse_roots_of_unity,
            roots_of_unity,
        },
    }
}

fn deserialize_blob(blob: *const Blob) -> Result<Vec<Fr>, C_KZG_RET> {
    let blob = unsafe { &*blob };

    blob
        .bytes
        .chunks(BYTES_PER_FIELD_ELEMENT)
        .map(|chunk| {
            let mut bytes = [0u8; BYTES_PER_FIELD_ELEMENT];
            bytes.copy_from_slice(chunk);
            bytes.reverse(); // conversion from be to le, as mcl works with le by default
            let mut tmp = Fr::default();
            let ret = tmp.deserialize(&bytes);
            if !ret {
                Err(C_KZG_RET_BADARGS)
            } else {
                Ok(tmp)
            }
            // fix for `test_verify_kzg_proof_batch__fails_with_incorrect_proof` c-kzg-4844 test
            //if let Ok(fr) = crate::eip_4844::bytes_to_bls_field(&bytes) {
            //    Ok(fr)
            //} else {
            //    Err(C_KZG_RET_BADARGS)
            //}
        })
        .collect::<Result<Vec<Fr>, C_KZG_RET>>()
}

/// # Safety
#[no_mangle]
pub unsafe extern "C" fn bytes_from_g1(out: *mut u8, in_: *const blst_p1) {
    assert!(crate::mcl_methods::init(crate::CurveType::BLS12_381));
    let b = cg1_to_g1(in_).to_bytes();
    let res = from_raw_parts_mut(out, b.len());
    res.copy_from_slice(&b);
}

/// # Safety
#[no_mangle]
pub unsafe extern "C" fn load_trusted_setup(
    out: *mut CKZGSettings,
    g1_bytes: *const u8,
    n1: usize,
    g2_bytes: *const u8,
    n2: usize,
) -> C_KZG_RET {
    assert!(crate::mcl_methods::init(crate::CurveType::BLS12_381));
    let mut mks = match crate::eip_4844::load_trusted_setup_from_bytes(
        from_raw_parts(g1_bytes, n1 * BYTES_PER_G1),
        from_raw_parts(g2_bytes, n2 * BYTES_PER_G2),
    ) {
        Ok(value) => value,
        Err(_) => return C_KZG_RET_BADARGS 
    };
    ks_to_cks(&mut mks, out);
    std::mem::forget(mks);
    C_KZG_RET_OK
}

/// # Safety
#[no_mangle]
pub unsafe extern "C" fn load_trusted_setup_file(
    out: *mut CKZGSettings,
    in_: *mut libc::FILE,
) -> C_KZG_RET {
    assert!(crate::mcl_methods::init(crate::CurveType::BLS12_381));

    let mut buf = vec![0u8; 1024 * 1024];
    let len: usize = libc::fread(buf.as_mut_ptr() as *mut libc::c_void, 1, buf.len(), in_);
    let s = String::from_utf8(buf[..len].to_vec()).unwrap();

    let (g1_bytes, g2_bytes) = match load_trusted_setup_string(&s) {
        Ok(value) => value,
        Err(_) => return C_KZG_RET_BADARGS
    };
    let mut mks = match
        crate::eip_4844::load_trusted_setup_from_bytes(g1_bytes.as_slice(), g2_bytes.as_slice()) {
            Ok(value) => value,
            Err(_) => return C_KZG_RET_BADARGS
        };
    ks_to_cks(&mut mks, out);
    std::mem::forget(mks);

    C_KZG_RET_OK
}

/// # Safety
#[no_mangle]
pub unsafe extern "C" fn free_trusted_setup(s: *mut CKZGSettings) {
    assert!(crate::mcl_methods::init(crate::CurveType::BLS12_381));
    drop(cks_to_ks(s));
}

/// # Safety
#[no_mangle]
pub unsafe extern "C" fn compute_blob_kzg_proof(
    out: *mut KZGProof,
    blob: *const Blob,
    commitment_bytes: *const Bytes48,
    s: &CKZGSettings,
) -> C_KZG_RET {
    assert!(crate::mcl_methods::init(crate::CurveType::BLS12_381));

    let deserialized_blob = deserialize_blob(blob);
    if deserialized_blob.is_err() {
        return deserialized_blob.err().unwrap();
    }
    let commitment_g1 = G1::from_bytes(&(*commitment_bytes).bytes);
    if commitment_g1.is_err() {
        return C_KZG_RET_BADARGS;
    }

    let ms = cks_to_ks(s);
    let proof = crate::eip_4844::compute_blob_kzg_proof(
        &deserialized_blob.unwrap(),
        &commitment_g1.unwrap(),
        &ms,
    );
    std::mem::forget(ms);

    if let Ok(proof) = proof {
        (*out).bytes = proof.to_bytes();
        C_KZG_RET_OK
    } else {
        C_KZG_RET_BADARGS
    }
}

/// # Safety
#[no_mangle]
pub unsafe extern "C" fn verify_blob_kzg_proof(
    ok: *mut bool,
    blob: *const Blob,
    commitment_bytes: *const Bytes48,
    proof_bytes: *const Bytes48,
    s: &CKZGSettings,
) -> C_KZG_RET {
    assert!(crate::mcl_methods::init(crate::CurveType::BLS12_381));

    let deserialized_blob = deserialize_blob(blob);
    if deserialized_blob.is_err() {
        return deserialized_blob.err().unwrap();
    }
    let commitment_g1 = G1::from_bytes(&(*commitment_bytes).bytes);
    let proof_g1 = G1::from_bytes(&(*proof_bytes).bytes);
    if commitment_g1.is_err() || proof_g1.is_err() {
        return C_KZG_RET_BADARGS;
    }

    let ms = cks_to_ks(s);
    let result = crate::eip_4844::verify_blob_kzg_proof(
        &deserialized_blob.unwrap(),
        &commitment_g1.unwrap(),
        &proof_g1.unwrap(),
        &ms,
    );
    std::mem::forget(ms);

    if let Ok(result) = result {
        *ok = result;
        C_KZG_RET_OK
    } else {
        C_KZG_RET_BADARGS
    }
}

/// # Safety
#[no_mangle]
pub unsafe extern "C" fn verify_blob_kzg_proof_batch(
    ok: *mut bool,
    blobs: *const Blob,
    commitments_bytes: *const Bytes48,
    proofs_bytes: *const Bytes48,
    n: usize,
    s: &CKZGSettings,
) -> C_KZG_RET {
    assert!(crate::mcl_methods::init(crate::CurveType::BLS12_381));

    let raw_blobs = from_raw_parts(blobs, n);
    let raw_commitments = from_raw_parts(commitments_bytes, n);
    let raw_proofs = from_raw_parts(proofs_bytes, n);

    let deserialized_blobs: Result<Vec<Vec<Fr>>, C_KZG_RET> = cfg_into_iter!(raw_blobs)
        .map(|raw_blob| deserialize_blob(raw_blob).map_err(|_| C_KZG_RET_BADARGS))
        .collect();

    let commitments_g1: Result<Vec<G1>, C_KZG_RET> = cfg_into_iter!(raw_commitments)
        .map(|raw_commitment| G1::from_bytes(&raw_commitment.bytes).map_err(|_| C_KZG_RET_BADARGS))
        .collect();

    let proofs_g1: Result<Vec<G1>, C_KZG_RET> = cfg_into_iter!(raw_proofs)
        .map(|raw_proof| G1::from_bytes(&raw_proof.bytes).map_err(|_| C_KZG_RET_BADARGS))
        .collect();

    if let (Ok(blobs), Ok(commitments), Ok(proofs)) =
        (deserialized_blobs, commitments_g1, proofs_g1)
    {
        let ms = cks_to_ks(s);
        let result = crate::eip_4844::verify_blob_kzg_proof_batch(
            blobs.as_slice(),
            &commitments,
            &proofs,
            &ms,
        );
        std::mem::forget(ms);

        if let Ok(result) = result {
            *ok = result;
            C_KZG_RET_OK
        } else {
            C_KZG_RET_BADARGS
        }
    } else {
        *ok = false;
        C_KZG_RET_BADARGS
    }
}

/// # Safety
#[no_mangle]
pub unsafe extern "C" fn blob_to_kzg_commitment(
    out: *mut KZGCommitment,
    blob: *const Blob,
    s: *const CKZGSettings,
) -> C_KZG_RET {
    assert!(crate::mcl_methods::init(crate::CurveType::BLS12_381));

    let deserialized_blob = deserialize_blob(blob);
    if deserialized_blob.is_err() {
        return deserialized_blob.err().unwrap();
    }
    let ms = cks_to_ks(s);
    let tmp = handle_ckzg_badargs!( crate::eip_4844::blob_to_kzg_commitment(&deserialized_blob.unwrap(), &ms));
    (*out).bytes = G1::to_bytes(&tmp);
    std::mem::forget(ms);

    C_KZG_RET_OK
}

/// # Safety
#[no_mangle]
pub unsafe extern "C" fn verify_kzg_proof(
    ok: *mut bool,
    commitment_bytes: *const Bytes48,
    z_bytes: *const Bytes32,
    y_bytes: *const Bytes32,
    proof_bytes: *const Bytes48,
    s: *const CKZGSettings,
) -> C_KZG_RET {
    assert!(crate::mcl_methods::init(crate::CurveType::BLS12_381));

    let frz = Fr::from_bytes(&(*z_bytes).bytes);
    let fry = Fr::from_bytes(&(*y_bytes).bytes);
    let g1commitment = G1::from_bytes(&(*commitment_bytes).bytes);
    let g1proof = G1::from_bytes(&(*proof_bytes).bytes);

    if frz.is_err() || fry.is_err() || g1commitment.is_err() || g1proof.is_err() {
        return C_KZG_RET_BADARGS;
    }

    let ms = cks_to_ks(s);
    let result = crate::eip_4844::verify_kzg_proof(
        &g1commitment.unwrap(),
        &frz.unwrap(),
        &fry.unwrap(),
        &g1proof.unwrap(),
        &ms,
    );
    std::mem::forget(ms);

    if let Ok(result) = result {
        *ok = result;
        C_KZG_RET_OK
    } else {
        C_KZG_RET_BADARGS
    }
}

/// # Safety
#[no_mangle]
pub unsafe extern "C" fn compute_kzg_proof(
    proof_out: *mut KZGProof,
    y_out: *mut Bytes32,
    blob: *const Blob,
    z_bytes: *const Bytes32,
    s: *const CKZGSettings,
) -> C_KZG_RET {
    assert!(crate::mcl_methods::init(crate::CurveType::BLS12_381));

    let deserialized_blob = deserialize_blob(blob);
    if deserialized_blob.is_err() {
        return deserialized_blob.err().unwrap();
    }
    let frz = Fr::from_bytes(&(*z_bytes).bytes);
    if frz.is_err() {
        return C_KZG_RET_BADARGS;
    }
    let ms = cks_to_ks(s);
    let (proof_out_tmp, fry_tmp) = handle_ckzg_badargs!( crate::eip_4844::compute_kzg_proof(&deserialized_blob.unwrap(), &frz.unwrap(), &ms) );
    (*proof_out).bytes = G1::to_bytes(&proof_out_tmp);
    (*y_out).bytes = Fr::to_bytes(&fry_tmp);
    std::mem::forget(ms);

    C_KZG_RET_OK
}
