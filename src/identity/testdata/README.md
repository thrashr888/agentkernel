# JWT test vectors

These public, non-production test vectors use a freshly generated 2048-bit RSA
key. Tokens were independently signed using OpenSSL (`genpkey -algorithm RSA
-pkeyopt rsa_keygen_bits:2048` and `dgst -sha256/-sha384/-sha512 -sign`). The private
key was discarded; only its public JWKS and signed test tokens are retained.
Valid tokens expire on 2100-01-01. Negative vectors are signed with the same key.

The AWS-LC JWT provider accepts 2048–8192-bit RSA keys, consistent with RFC 7518's
2048-bit minimum. Legacy issuers using smaller keys must rotate to compliant keys.
