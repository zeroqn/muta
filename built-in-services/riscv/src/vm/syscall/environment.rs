//! Environmental Information
use ckb_vm::instructions::Register;
use ckb_vm::memory::Memory;
use log::error;
use protocol::{types::ServiceContext, Bytes};

use crate::vm::syscall::common::{get_arr, invalid_ecall};
use crate::vm::syscall::convention::{
    SYSCODE_ADDRESS, SYSCODE_BLOCK_HEIGHT, SYSCODE_CALLER, SYSCODE_CYCLE_LIMIT,
    SYSCODE_CYCLE_PRICE, SYSCODE_CYCLE_USED, SYSCODE_EMIT_EVENT, SYSCODE_EXTRA, SYSCODE_IS_INIT,
    SYSCODE_ORIGIN, SYSCODE_TIMESTAMP, SYSCODE_TX_HASH, SYSCODE_TX_NONCE,
};
use crate::InterpreterParams;

pub struct SyscallEnvironment {
    context: ServiceContext,
    iparams: InterpreterParams,
}

impl SyscallEnvironment {
    pub fn new(context: ServiceContext, iparams: InterpreterParams) -> Self {
        Self { context, iparams }
    }
}

impl<Mac: ckb_vm::SupportMachine> ckb_vm::Syscalls<Mac> for SyscallEnvironment {
    fn initialize(&mut self, _machine: &mut Mac) -> Result<(), ckb_vm::Error> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut Mac) -> Result<bool, ckb_vm::Error> {
        let code = machine.registers()[ckb_vm::registers::A7].to_u64();

        match code {
            SYSCODE_ADDRESS => {
                let ptr = machine.registers()[ckb_vm::registers::A0].to_u64();
                if ptr == 0 {
                    return Err(invalid_ecall(code));
                }

                machine
                    .memory_mut()
                    .store_bytes(ptr, self.iparams.address.as_hex().as_ref())?;

                machine.set_register(ckb_vm::registers::A0, Mac::REG::from_u8(0));
                Ok(true)
            }
            SYSCODE_CYCLE_LIMIT => {
                let gaslimit_byte = self.context.get_cycles_limit();
                machine.set_register(ckb_vm::registers::A0, Mac::REG::from_u64(gaslimit_byte));
                Ok(true)
            }
            SYSCODE_CYCLE_PRICE => {
                let cycle_price = self.context.get_cycles_price();
                machine.set_register(ckb_vm::registers::A0, Mac::REG::from_u64(cycle_price));
                Ok(true)
            }
            SYSCODE_CYCLE_USED => {
                let cycles_used = self.context.get_cycles_used();
                machine.set_register(ckb_vm::registers::A0, Mac::REG::from_u64(cycles_used));
                Ok(true)
            }
            SYSCODE_IS_INIT => {
                let is_init = if self.iparams.is_init { 1u8 } else { 0u8 };
                machine.set_register(ckb_vm::registers::A0, Mac::REG::from_u8(is_init));
                Ok(true)
            }
            SYSCODE_ORIGIN => {
                let ptr = machine.registers()[ckb_vm::registers::A0].to_u64();
                if ptr == 0 {
                    return Err(invalid_ecall(code));
                }

                machine
                    .memory_mut()
                    .store_bytes(ptr, self.context.get_caller().as_hex().as_ref())?;

                machine.set_register(ckb_vm::registers::A0, Mac::REG::from_u8(0));
                Ok(true)
            }
            SYSCODE_CALLER => {
                let ptr = machine.registers()[ckb_vm::registers::A0].to_u64();
                if ptr == 0 {
                    return Err(invalid_ecall(code));
                }

                let caller = self
                    .context
                    .get_extra()
                    .unwrap_or_else(|| Bytes::from(self.context.get_caller().as_hex()));

                machine.memory_mut().store_bytes(ptr, &caller)?;
                machine.set_register(ckb_vm::registers::A0, Mac::REG::from_u8(0));
                Ok(true)
            }
            SYSCODE_BLOCK_HEIGHT => {
                let block_height = self.context.get_current_height();
                machine.set_register(ckb_vm::registers::A0, Mac::REG::from_u64(block_height));
                Ok(true)
            }
            SYSCODE_EXTRA => {
                let ptr = machine.registers()[ckb_vm::registers::A0].to_u64();
                let len_ptr = machine.registers()[ckb_vm::registers::A1].to_u64();

                if let Some(extra) = self.context.get_extra() {
                    if ptr != 0 {
                        machine.memory_mut().store_bytes(ptr, &extra)?;
                    }
                    if len_ptr != 0 {
                        let extra_len = (extra.len() as u64).to_le_bytes();
                        machine.memory_mut().store_bytes(len_ptr, &extra_len)?;
                    }

                    machine.set_register(ckb_vm::registers::A0, Mac::REG::from_u8(0));
                } else {
                    machine.set_register(ckb_vm::registers::A0, Mac::REG::from_u8(1));
                }
                Ok(true)
            }
            SYSCODE_TIMESTAMP => {
                let timestamp = self.context.get_timestamp();
                machine.set_register(ckb_vm::registers::A0, Mac::REG::from_u64(timestamp));
                Ok(true)
            }
            SYSCODE_EMIT_EVENT => {
                let ptr = machine.registers()[ckb_vm::registers::A0].to_u64();
                let len = machine.registers()[ckb_vm::registers::A1].to_u64();
                let msg_bytes = get_arr(machine, ptr, len)?;

                if let Ok(msg) = String::from_utf8(msg_bytes) {
                    // Note: Right now, emit event is infallible
                    if let Err(e) = self.context.emit_event(msg) {
                        error!("impossible emit event failed {}", e);
                    }

                    machine.set_register(ckb_vm::registers::A0, Mac::REG::from_u8(0));
                } else {
                    // TODO: throw error
                    machine.set_register(ckb_vm::registers::A0, Mac::REG::from_u8(1));
                }

                Ok(true)
            }
            SYSCODE_TX_HASH => {
                let ptr = machine.registers()[ckb_vm::registers::A0].to_u64();
                if ptr == 0 {
                    return Err(invalid_ecall(code));
                }

                if let Some(tx_hash) = self.context.get_tx_hash().map(|h| h.as_hex()) {
                    machine.memory_mut().store_bytes(ptr, tx_hash.as_ref())?;
                    machine.set_register(ckb_vm::registers::A0, Mac::REG::from_u8(0));
                } else {
                    machine.set_register(ckb_vm::registers::A0, Mac::REG::from_u8(1));
                }

                Ok(true)
            }
            SYSCODE_TX_NONCE => {
                let ptr = machine.registers()[ckb_vm::registers::A0].to_u64();
                if ptr == 0 {
                    return Err(invalid_ecall(code));
                }

                if let Some(nonce) = self.context.get_nonce().map(|n| n.as_hex()) {
                    machine.memory_mut().store_bytes(ptr, nonce.as_ref())?;
                    machine.set_register(ckb_vm::registers::A0, Mac::REG::from_u8(0));
                } else {
                    machine.set_register(ckb_vm::registers::A0, Mac::REG::from_u8(1));
                }

                Ok(true)
            }

            _ => Ok(false),
        }
    }
}
