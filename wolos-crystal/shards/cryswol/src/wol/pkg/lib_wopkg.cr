# SPDX-License-Identifier: GPL-2.0-or-later
# Generated from crates/wopkg/include/wopkg.h by crates/wol-abi/tests/bindings.rs; do not edit.
# Regenerate: WOL_BLESS=1 cargo test -p wol-abi --test bindings
@[Link("wopkg")]
lib LibWopkg
  WOPKG_VERSION_MAJOR         = 1
  WOPKG_VERSION_MINOR         = 1
  WOPKG_VERSION_PATCH         = 0
  WOPKG_OK                    = 0
  WOPKG_ERR_INVALID_ARGUMENT  = 1
  WOPKG_ERR_PERMISSION_DENIED = 2
  WOPKG_ERR_UNAVAILABLE       = 3
  WOPKG_ERR_INTERNAL          = 4
  WOPKG_ERR_UNTRUSTED         = 5
  WOPKG_ERR_NOT_FOUND         = 6

  alias Status = Int32

  # Handles are opaque: the field exists only because a Crystal struct cannot be empty.
  struct Context
    opaque : UInt8
  end

  struct Error
    opaque : UInt8
  end

  struct String
    opaque : UInt8
  end

  alias ProgressFn = UInt8*, LibC::SizeT, Void* -> Void

  fun wopkg_version : UInt32
  fun wopkg_version_string : UInt8*
  fun wopkg_string_get_bytes(string : String*, len : LibC::SizeT*) : UInt8*
  fun wopkg_string_release(string : String*)
  fun wopkg_error_get_code(error : Error*) : Status
  fun wopkg_error_get_message(error : Error*, len : LibC::SizeT*) : UInt8*
  fun wopkg_error_release(error : Error*)
  fun wopkg_context_create(config : UInt8*, len : LibC::SizeT, out_ : Context**, error : Error**) : Status
  fun wopkg_context_release(context : Context*)
  fun wopkg_index_update(context : Context*, request : UInt8*, len : LibC::SizeT, progress : ProgressFn, user_data : Void*, out_ : String**, error : Error**) : Status
  fun wopkg_search(context : Context*, query : UInt8*, len : LibC::SizeT, limit : UInt32, out_ : String**, error : Error**) : Status
  fun wopkg_info(context : Context*, name : UInt8*, len : LibC::SizeT, out_ : String**, error : Error**) : Status
  fun wopkg_complete(context : Context*, prefix : UInt8*, len : LibC::SizeT, limit : UInt32, out_ : String**, error : Error**) : Status
  fun wopkg_installed(context : Context*, out_ : String**, error : Error**) : Status
  fun wopkg_fetch(context : Context*, request : UInt8*, len : LibC::SizeT, progress : ProgressFn, user_data : Void*, out_ : String**, error : Error**) : Status
  fun wopkg_verify(context : Context*, request : UInt8*, len : LibC::SizeT, out_ : String**, error : Error**) : Status
  fun wopkg_appimage_inspect(context : Context*, path : UInt8*, len : LibC::SizeT, out_ : String**, error : Error**) : Status
  fun wopkg_appimage_install(context : Context*, request : UInt8*, len : LibC::SizeT, progress : ProgressFn, user_data : Void*, out_ : String**, error : Error**) : Status
  fun wopkg_appimage_remove(context : Context*, name : UInt8*, len : LibC::SizeT, out_ : String**, error : Error**) : Status
  fun wopkg_appimage_update(context : Context*, request : UInt8*, len : LibC::SizeT, progress : ProgressFn, user_data : Void*, out_ : String**, error : Error**) : Status
  fun wopkg_history(context : Context*, out_ : String**, error : Error**) : Status
  fun wopkg_history_record(context : Context*, entry : UInt8*, len : LibC::SizeT, out_ : String**, error : Error**) : Status
  fun wopkg_rollback(context : Context*, id : UInt8*, len : LibC::SizeT, out_ : String**, error : Error**) : Status
  fun wopkg_system_call(context : Context*, request : UInt8*, len : LibC::SizeT, error : Error**) : Status
  fun wopkg_release_plan(context : Context*, request : UInt8*, len : LibC::SizeT, out_ : String**, error : Error**) : Status
  fun wopkg_shard_index(context : Context*, request : UInt8*, len : LibC::SizeT, out_ : String**, error : Error**) : Status
  fun wopkg_shard_pin(context : Context*, request : UInt8*, len : LibC::SizeT, out_ : String**, error : Error**) : Status
  fun wopkg_shard_check(context : Context*, request : UInt8*, len : LibC::SizeT, out_ : String**, error : Error**) : Status
end
