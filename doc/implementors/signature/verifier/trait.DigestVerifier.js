(function() {var implementors = {
"ecdsa":[["impl&lt;C, D&gt; <a class=\"trait\" href=\"signature/verifier/trait.DigestVerifier.html\" title=\"trait signature::verifier::DigestVerifier\">DigestVerifier</a>&lt;D, <a class=\"struct\" href=\"ecdsa/struct.Signature.html\" title=\"struct ecdsa::Signature\">Signature</a>&lt;C&gt;&gt; for <a class=\"struct\" href=\"ecdsa/struct.VerifyingKey.html\" title=\"struct ecdsa::VerifyingKey\">VerifyingKey</a>&lt;C&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;C: <a class=\"trait\" href=\"ecdsa/trait.PrimeCurve.html\" title=\"trait ecdsa::PrimeCurve\">PrimeCurve</a> + <a class=\"trait\" href=\"elliptic_curve/arithmetic/trait.ProjectiveArithmetic.html\" title=\"trait elliptic_curve::arithmetic::ProjectiveArithmetic\">ProjectiveArithmetic</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;D: <a class=\"trait\" href=\"digest/digest/trait.Digest.html\" title=\"trait digest::digest::Digest\">Digest</a> + <a class=\"trait\" href=\"digest/trait.FixedOutput.html\" title=\"trait digest::FixedOutput\">FixedOutput</a>&lt;OutputSize = <a class=\"type\" href=\"elliptic_curve/type.FieldSize.html\" title=\"type elliptic_curve::FieldSize\">FieldSize</a>&lt;C&gt;&gt;,<br>&nbsp;&nbsp;&nbsp;&nbsp;<a class=\"type\" href=\"elliptic_curve/type.AffinePoint.html\" title=\"type elliptic_curve::AffinePoint\">AffinePoint</a>&lt;C&gt;: <a class=\"trait\" href=\"ecdsa/hazmat/trait.VerifyPrimitive.html\" title=\"trait ecdsa::hazmat::VerifyPrimitive\">VerifyPrimitive</a>&lt;C&gt;,<br>&nbsp;&nbsp;&nbsp;&nbsp;<a class=\"type\" href=\"elliptic_curve/scalar/type.Scalar.html\" title=\"type elliptic_curve::scalar::Scalar\">Scalar</a>&lt;C&gt;: <a class=\"trait\" href=\"elliptic_curve/ops/trait.Reduce.html\" title=\"trait elliptic_curve::ops::Reduce\">Reduce</a>&lt;C::<a class=\"associatedtype\" href=\"elliptic_curve/trait.Curve.html#associatedtype.UInt\" title=\"type elliptic_curve::Curve::UInt\">UInt</a>&gt;,<br>&nbsp;&nbsp;&nbsp;&nbsp;<a class=\"type\" href=\"ecdsa/type.SignatureSize.html\" title=\"type ecdsa::SignatureSize\">SignatureSize</a>&lt;C&gt;: <a class=\"trait\" href=\"generic_array/trait.ArrayLength.html\" title=\"trait generic_array::ArrayLength\">ArrayLength</a>&lt;<a class=\"primitive\" href=\"https://doc.rust-lang.org/1.66.0/std/primitive.u8.html\">u8</a>&gt;,</span>"]],
"k256":[["impl&lt;D&gt; <a class=\"trait\" href=\"signature/verifier/trait.DigestVerifier.html\" title=\"trait signature::verifier::DigestVerifier\">DigestVerifier</a>&lt;D, <a class=\"struct\" href=\"ecdsa/struct.Signature.html\" title=\"struct ecdsa::Signature\">Signature</a>&lt;<a class=\"struct\" href=\"k256/struct.Secp256k1.html\" title=\"struct k256::Secp256k1\">Secp256k1</a>&gt;&gt; for <a class=\"struct\" href=\"k256/ecdsa/struct.VerifyingKey.html\" title=\"struct k256::ecdsa::VerifyingKey\">VerifyingKey</a><span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;D: <a class=\"trait\" href=\"digest/digest/trait.Digest.html\" title=\"trait digest::digest::Digest\">Digest</a> + <a class=\"trait\" href=\"digest/trait.FixedOutput.html\" title=\"trait digest::FixedOutput\">FixedOutput</a>&lt;OutputSize = <a class=\"type\" href=\"typenum/generated/consts/type.U32.html\" title=\"type typenum::generated::consts::U32\">U32</a>&gt;,</span>"],["impl&lt;D&gt; <a class=\"trait\" href=\"signature/verifier/trait.DigestVerifier.html\" title=\"trait signature::verifier::DigestVerifier\">DigestVerifier</a>&lt;D, <a class=\"struct\" href=\"k256/ecdsa/recoverable/struct.Signature.html\" title=\"struct k256::ecdsa::recoverable::Signature\">Signature</a>&gt; for <a class=\"struct\" href=\"k256/ecdsa/struct.VerifyingKey.html\" title=\"struct k256::ecdsa::VerifyingKey\">VerifyingKey</a><span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;D: <a class=\"trait\" href=\"digest/digest/trait.Digest.html\" title=\"trait digest::digest::Digest\">Digest</a> + <a class=\"trait\" href=\"digest/trait.FixedOutput.html\" title=\"trait digest::FixedOutput\">FixedOutput</a>&lt;OutputSize = <a class=\"type\" href=\"typenum/generated/consts/type.U32.html\" title=\"type typenum::generated::consts::U32\">U32</a>&gt;,</span>"],["impl&lt;D&gt; <a class=\"trait\" href=\"signature/verifier/trait.DigestVerifier.html\" title=\"trait signature::verifier::DigestVerifier\">DigestVerifier</a>&lt;D, <a class=\"struct\" href=\"k256/schnorr/struct.Signature.html\" title=\"struct k256::schnorr::Signature\">Signature</a>&gt; for <a class=\"struct\" href=\"k256/schnorr/struct.VerifyingKey.html\" title=\"struct k256::schnorr::VerifyingKey\">VerifyingKey</a><span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;D: <a class=\"trait\" href=\"digest/digest/trait.Digest.html\" title=\"trait digest::digest::Digest\">Digest</a> + <a class=\"trait\" href=\"digest/trait.FixedOutput.html\" title=\"trait digest::FixedOutput\">FixedOutput</a>&lt;OutputSize = <a class=\"type\" href=\"typenum/generated/consts/type.U32.html\" title=\"type typenum::generated::consts::U32\">U32</a>&gt;,</span>"]]
};if (window.register_implementors) {window.register_implementors(implementors);} else {window.pending_implementors = implementors;}})()