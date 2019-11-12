//! Identity Based Encryption Waters scheme on the [BLS12-381 pairing-friendly elliptic curve](https://github.com/zkcrypto/bls12_381).
//!  * From: "[Efficient Identity-Based Encryption Without Random Oracles](https://link.springer.com/chapter/10.1007/11426639_7)"
//!  * Published in: EUROCRYPT, 2005
//!
//! Uses [SHA3-256](https://crates.io/crates/tiny-keccak) for hashing to identities.
//!
//! The structure of the byte serialisation of the various datastructures is not guaranteed
//! to remain constant between releases of this library.
//! All operations in this library are implemented to run in constant time.

use arrayref::{array_mut_ref, array_ref, array_refs, mut_array_refs};
use rand::Rng;
use subtle::{Choice, ConditionallySelectable, CtOption};

use crate::bls12_381::{G1Affine, G1Projective, G2Affine, Gt};
use crate::util::*;

const HASH_BIT_LEN: usize = 256;
const HASH_BYTE_LEN: usize = HASH_BIT_LEN / 8;

const CHUNKS: usize = HASH_BIT_LEN;

const PARAMETERSIZE: usize = CHUNKS * 48;
const PUBLICKEYSIZE: usize = 2 * 48 + 2 * 96 + PARAMETERSIZE;

/// Public key parameters used for entanglement with identities.
struct Parameters([G1Affine; CHUNKS]);

/// Public key parameters generated by the PKG used to encrypt messages.
#[derive(Clone, Copy, PartialEq)]
pub struct PublicKey {
    g: G2Affine,
    g1: G1Affine,
    g2: G2Affine,
    uprime: G1Affine,
    u: Parameters,
}

/// Secret key parameter generated by the PKG used to extract user secret keys.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SecretKey {
    g1prime: G1Affine,
}

/// Points on the paired curves that form the user secret key.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct UserSecretKey {
    d1: G1Affine,
    d2: G2Affine,
}

/// Field parameters for an identity.
///
/// Effectively a hash of an identity, mapped to the curve field.
/// Together with the public key parameters generated by the PKG forms the user public key.
pub struct Identity([u8; HASH_BYTE_LEN]);

/// A point on the paired curve that can be encrypted and decrypted.
///
/// You can use the byte representation to derive an AES key.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Message(Gt);

/// Encrypted message. Can only be decrypted with an user secret key.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CipherText {
    c1: Gt,
    c2: G2Affine,
    c3: G1Affine,
}

/// Generate a keypair used by the Private Key Generator (PKG).
pub fn setup<R: Rng>(rng: &mut R) -> (PublicKey, SecretKey) {
    let g: G2Affine = rand_g2(rng).into();

    let alpha = rand_scalar(rng);
    let g2 = (g * alpha).into();

    let g1 = rand_g1(rng).into();
    let uprime = rand_g1(rng).into();

    let mut u = Parameters([G1Affine::default(); CHUNKS]);
    for ui in u.0.iter_mut() {
        *ui = rand_g1(rng).into();
    }

    let pk = PublicKey {
        g,
        g1,
        g2,
        uprime,
        u,
    };

    let g1prime: G1Affine = (g1 * alpha).into();

    let sk = SecretKey { g1prime };

    (pk, sk)
}

/// Common operation used in extraction and encryption to entangle
/// PublicKey with Identity into a point on G1.
fn entangle(pk: &PublicKey, v: &Identity) -> G1Projective {
    let mut ucoll: G1Projective = pk.uprime.into();
    for (ui, vi) in pk.u.0.iter().zip(bits(&v.0)) {
        ucoll = G1Projective::conditional_select(&ucoll, &(ui + ucoll), vi);
    }
    ucoll
}

/// Extract an user secret key for a given identity.
pub fn extract_usk<R: Rng>(
    pk: &PublicKey,
    sk: &SecretKey,
    v: &Identity,
    rng: &mut R,
) -> UserSecretKey {
    let r = rand_scalar(rng);
    let ucoll = entangle(pk, v);
    let d1 = (sk.g1prime + (ucoll * r)).into();
    let d2 = (pk.g * r).into();

    UserSecretKey { d1, d2 }
}

/// Encrypt a message using the PKG public key and an identity.
pub fn encrypt<R: Rng>(pk: &PublicKey, v: &Identity, m: &Message, rng: &mut R) -> CipherText {
    let t = rand_scalar(rng);

    let c3coll = entangle(pk, v);
    let c1 = crate::bls12_381::pairing(&pk.g1, &pk.g2) * t + m.0;
    let c2 = (pk.g * t).into();
    let c3 = (c3coll * t).into();

    CipherText { c1, c2, c3 }
}

/// Decrypt ciphertext to a message using a user secret key.
pub fn decrypt(usk: &UserSecretKey, c: &CipherText) -> Message {
    let num = crate::bls12_381::pairing(&c.c3, &usk.d2);
    let dem = crate::bls12_381::pairing(&usk.d1, &c.c2);

    let m = c.c1 + num - dem;
    Message(m)
}

impl PublicKey {
    pub fn to_bytes(&self) -> [u8; PUBLICKEYSIZE] {
        let mut res = [0u8; PUBLICKEYSIZE];
        let (g, g1, g2, uprime, u) = mut_array_refs![&mut res, 96, 48, 96, 48, PARAMETERSIZE];
        *g = self.g.to_compressed();
        *g1 = self.g1.to_compressed();
        *g2 = self.g2.to_compressed();
        *uprime = self.uprime.to_compressed();
        *u = self.u.to_bytes();
        res
    }

    pub fn from_bytes(bytes: &[u8; PUBLICKEYSIZE]) -> CtOption<Self> {
        let (g, g1, g2, uprime, u) = array_refs![bytes, 96, 48, 96, 48, PARAMETERSIZE];

        let g = G2Affine::from_compressed(g);
        let g1 = G1Affine::from_compressed(g1);
        let g2 = G2Affine::from_compressed(g2);
        let uprime = G1Affine::from_compressed(uprime);
        let u = Parameters::from_bytes(u);

        g.and_then(|g| {
            g1.and_then(|g1| {
                g2.and_then(|g2| {
                    uprime.and_then(|uprime| {
                        u.map(|u| PublicKey {
                            g,
                            g1,
                            g2,
                            uprime,
                            u,
                        })
                    })
                })
            })
        })
    }
}

impl SecretKey {
    pub fn to_bytes(&self) -> [u8; 48] {
        self.g1prime.to_compressed()
    }

    pub fn from_bytes(bytes: &[u8; 48]) -> CtOption<Self> {
        G1Affine::from_compressed(bytes).map(|g1prime| SecretKey { g1prime })
    }
}

impl UserSecretKey {
    pub fn to_bytes(&self) -> [u8; 144] {
        let mut res = [0u8; 144];
        let (d1, d2) = mut_array_refs![&mut res, 48, 96];
        *d1 = self.d1.to_compressed();
        *d2 = self.d2.to_compressed();
        res
    }

    pub fn from_bytes(bytes: &[u8; 144]) -> CtOption<Self> {
        let (d1, d2) = array_refs![bytes, 48, 96];

        let d1 = G1Affine::from_compressed(d1);
        let d2 = G2Affine::from_compressed(d2);

        d1.and_then(|d1| d2.map(|d2| UserSecretKey { d1, d2 }))
    }
}

impl Message {
    /// Generate a random point on the paired curve.
    pub fn generate<R: Rng>(rng: &mut R) -> Self {
        Self(rand_gt(rng))
    }

    pub fn to_bytes(&self) -> [u8; 288] {
        self.0.to_compressed()
    }

    pub fn from_bytes(bytes: &[u8; 288]) -> CtOption<Self> {
        Gt::from_compressed(bytes).map(Message)
    }
}

impl Parameters {
    pub fn to_bytes(&self) -> [u8; PARAMETERSIZE] {
        let mut res = [0u8; PARAMETERSIZE];
        for i in 0..CHUNKS {
            *array_mut_ref![&mut res, i * 48, 48] = self.0[i].to_compressed();
        }
        res
    }

    pub fn from_bytes(bytes: &[u8; PARAMETERSIZE]) -> CtOption<Self> {
        let mut res = [G1Affine::default(); CHUNKS];
        let mut is_some = Choice::from(1u8);
        for i in 0..CHUNKS {
            is_some &= G1Affine::from_compressed(array_ref![bytes, i * 48, 48])
                .map(|s| {
                    res[i] = s;
                })
                .is_some();
        }
        CtOption::new(Parameters(res), is_some)
    }
}

impl ConditionallySelectable for Parameters {
    fn conditional_select(a: &Self, b: &Self, choice: Choice) -> Self {
        let mut res = [G1Affine::default(); CHUNKS];
        for (i, (ai, bi)) in a.0.iter().zip(b.0.iter()).enumerate() {
            res[i] = G1Affine::conditional_select(&ai, &bi, choice);
        }
        Parameters(res)
    }
}

impl Clone for Parameters {
    fn clone(&self) -> Self {
        let mut res = [G1Affine::default(); CHUNKS];
        for (src, dst) in self.0.iter().zip(res.as_mut().iter_mut()) {
            *dst = *src;
        }
        Parameters(res)
    }
}

impl Copy for Parameters {}

impl PartialEq for Parameters {
    fn eq(&self, rhs: &Self) -> bool {
        self.0.iter().zip(rhs.0.iter()).all(|(x, y)| x.eq(y))
    }
}

impl Default for Parameters {
    fn default() -> Self {
        Parameters([G1Affine::default(); CHUNKS])
    }
}

impl Identity {
    /// Hash a byte slice to a set of Identity parameters, which acts as a user public key.
    /// Uses sha3-256 internally.
    pub fn derive(b: &[u8]) -> Identity {
        Identity(tiny_keccak::sha3_256(b))
    }

    /// Hash a string slice to a set of Identity parameters.
    /// Directly converts characters to UTF-8 byte representation.
    pub fn derive_str(s: &str) -> Identity {
        Self::derive(s.as_bytes())
    }
}

impl Clone for Identity {
    fn clone(&self) -> Self {
        let mut res = [u8::default(); HASH_BYTE_LEN];
        for (src, dst) in self.0.iter().zip(res.as_mut().iter_mut()) {
            *dst = *src;
        }
        Identity(res)
    }
}

impl Copy for Identity {}

impl CipherText {
    pub fn to_bytes(&self) -> [u8; 432] {
        let mut res = [0u8; 432];
        let (c1, c2, c3) = mut_array_refs![&mut res, 288, 96, 48];
        *c1 = self.c1.to_compressed();
        *c2 = self.c2.to_compressed();
        *c3 = self.c3.to_compressed();
        res
    }

    pub fn from_bytes(bytes: &[u8; 432]) -> CtOption<Self> {
        let (c1, c2, c3) = array_refs![bytes, 288, 96, 48];

        let c1 = Gt::from_compressed(c1);
        let c2 = G2Affine::from_compressed(c2);
        let c3 = G1Affine::from_compressed(c3);

        c1.and_then(|c1| c2.and_then(|c2| c3.map(|c3| CipherText { c1, c2, c3 })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &'static str = "email:w.geraedts@sarif.nl";

    #[allow(dead_code)]
    struct DefaultSubResults {
        kid: Identity,
        m: Message,
        pk: PublicKey,
        sk: SecretKey,
        usk: UserSecretKey,
        c: CipherText,
    }

    fn perform_default() -> DefaultSubResults {
        let mut rng = rand::thread_rng();

        let id = ID.as_bytes();
        let kid = Identity::derive(id);

        let m = Message::generate(&mut rng);

        let (pk, sk) = setup(&mut rng);
        let usk = extract_usk(&pk, &sk, &kid, &mut rng);

        let c = encrypt(&pk, &kid, &m, &mut rng);

        DefaultSubResults {
            kid,
            m,
            pk,
            sk,
            usk,
            c,
        }
    }

    #[test]
    fn eq_encrypt_decrypt() {
        let results = perform_default();
        let m2 = decrypt(&results.usk, &results.c);

        assert_eq!(results.m, m2);
    }

    #[test]
    fn eq_serialize_deserialize() {
        let result = perform_default();

        assert_eq!(result.m, Message::from_bytes(&result.m.to_bytes()).unwrap());
        assert!(result.pk == PublicKey::from_bytes(&result.pk.to_bytes()).unwrap());
        assert_eq!(
            result.sk,
            SecretKey::from_bytes(&result.sk.to_bytes()).unwrap()
        );
        assert_eq!(
            result.usk,
            UserSecretKey::from_bytes(&result.usk.to_bytes()).unwrap()
        );
        assert_eq!(
            result.c,
            CipherText::from_bytes(&result.c.to_bytes()).unwrap()
        );
    }
}
