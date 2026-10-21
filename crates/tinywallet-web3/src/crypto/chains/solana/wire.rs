//! Hand-built Solana wire format, so the crate does not pull in `solana-sdk`
//! (which transitively brings in ~150 crates).
//!
//! References:
//! - <https://docs.solana.com/developing/programming-model/transactions>
//! - <https://docs.solana.com/developing/programming-model/runtime#compact-u16>
//! - <https://spl.solana.com/token>

pub(super) use tinywallet_bus::solana::{
    associated_token_address as associated_token_account, pubkey_from_base58 as b58_to_pubkey,
    pubkey_to_base58 as pubkey_to_b58, token_program_id,
};

/// System Program ID (all zeros).
pub(super) const SYSTEM_PROGRAM_ID: [u8; 32] = [0u8; 32];

/// Solana compact-u16 (shortvec) encoding.
pub(super) fn encode_shortvec(value: u16) -> Vec<u8> {
    let mut out = Vec::new();
    let mut v = u32::from(value);
    loop {
        // Masked to 7 bits, so the narrowing cannot lose information.
        let mut byte = u8::try_from(v & 0x7f).unwrap_or(0x7f);
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return out;
        }
        byte |= 0x80;
        out.push(byte);
    }
}

/// Decode a Solana compact-u16 (shortvec). Returns `(value, bytes_consumed)`.
pub(super) fn decode_shortvec(bytes: &[u8]) -> Result<(u16, usize), String> {
    let mut value: u32 = 0;
    let mut shift = 0u32;
    for (i, byte) in bytes.iter().enumerate() {
        if i >= 3 {
            return Err("shortvec too long".to_string());
        }
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            let decoded =
                u16::try_from(value).map_err(|_| "shortvec exceeds u16 range".to_string())?;
            return Ok((decoded, i + 1));
        }
        shift += 7;
    }
    Err("shortvec truncated".to_string())
}

/// A shortvec length prefix for a slice length.
fn shortvec_len(len: usize) -> Vec<u8> {
    // Instruction, account and key counts are bounded far below `u16::MAX` by
    // the transaction size limit; saturating keeps an absurd input encodable
    // rather than panicking.
    encode_shortvec(u16::try_from(len).unwrap_or(u16::MAX))
}

#[derive(Debug, Clone)]
struct CompiledInstruction {
    program_id_index: u8,
    accounts: Vec<u8>,
    data: Vec<u8>,
}

fn encode_compiled_instruction(ins: &CompiledInstruction) -> Vec<u8> {
    let mut out = vec![ins.program_id_index];
    out.extend(shortvec_len(ins.accounts.len()));
    out.extend(&ins.accounts);
    out.extend(shortvec_len(ins.data.len()));
    out.extend(&ins.data);
    out
}

fn encode_message(
    header: [u8; 3],
    account_keys: &[[u8; 32]],
    recent_blockhash: &[u8; 32],
    instructions: &[CompiledInstruction],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(header);
    out.extend(shortvec_len(account_keys.len()));
    for key in account_keys {
        out.extend(key);
    }
    out.extend(recent_blockhash);
    out.extend(shortvec_len(instructions.len()));
    for ins in instructions {
        out.extend(encode_compiled_instruction(ins));
    }
    out
}

/// A native SOL transfer message.
pub(super) fn build_native_transfer_message(
    from: [u8; 32],
    to: [u8; 32],
    lamports: u64,
    recent_blockhash: [u8; 32],
) -> Vec<u8> {
    // accounts: [from (signer, writable), to (writable), system_program (read-only)]
    let account_keys = [from, to, SYSTEM_PROGRAM_ID];
    // header: 1 required sig, 0 readonly signed, 1 readonly unsigned (system program)
    let header = [1u8, 0u8, 1u8];
    let mut data = vec![2u8, 0u8, 0u8, 0u8]; // SystemInstruction::Transfer = 2
    data.extend(lamports.to_le_bytes());
    let ins = CompiledInstruction {
        program_id_index: 2,
        accounts: vec![0, 1],
        data,
    };
    encode_message(header, &account_keys, &recent_blockhash, &[ins])
}

/// An SPL token transfer message between two Associated Token Accounts.
pub(super) fn build_spl_transfer_message(
    from_owner: [u8; 32],
    src_ata: [u8; 32],
    dst_ata: [u8; 32],
    amount: u64,
    recent_blockhash: [u8; 32],
) -> Result<Vec<u8>, String> {
    let token_program = token_program_id()?;
    // accounts:
    //  0: from_owner (signer, writable: pays the fee)
    //  1: src_ata (writable)
    //  2: dst_ata (writable)
    //  3: token_program (readonly, unsigned)
    let account_keys = [from_owner, src_ata, dst_ata, token_program];
    let header = [1u8, 0u8, 1u8];
    let mut data = vec![3u8]; // SPL Token instruction: Transfer = 3
    data.extend(amount.to_le_bytes());
    let ins = CompiledInstruction {
        program_id_index: 3,
        accounts: vec![1, 2, 0], // src, dst, owner(signer)
        data,
    };
    Ok(encode_message(
        header,
        &account_keys,
        &recent_blockhash,
        &[ins],
    ))
}

#[cfg(test)]
#[path = "wire/wire_tests.rs"]
mod test;
