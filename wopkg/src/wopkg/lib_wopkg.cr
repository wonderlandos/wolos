# SPDX-License-Identifier: BSD-2-Clause
# libwopkg C ABI (crates/wopkg/include/wopkg.h, ADR 0021).
@[Link("wopkg")]
lib LibWopkg
  alias Status = Int32
  alias Context = Void*
  alias Error = Void*
  alias WString = Void*
  alias ProgressFn = (UInt8*, LibC::SizeT, Void*) -> Void

  fun version = wopkg_version : UInt32
  fun version_string = wopkg_version_string : UInt8*
  fun string_get_bytes = wopkg_string_get_bytes(s : WString, len : LibC::SizeT*) : UInt8*
  fun string_release = wopkg_string_release(s : WString)
  fun error_get_code = wopkg_error_get_code(e : Error) : Status
  fun error_get_message = wopkg_error_get_message(e : Error, len : LibC::SizeT*) : UInt8*
  fun error_release = wopkg_error_release(e : Error)
  fun context_create = wopkg_context_create(config : UInt8*, len : LibC::SizeT, out : Context*, error : Error*) : Status
  fun context_release = wopkg_context_release(c : Context)
  fun index_update = wopkg_index_update(c : Context, req : UInt8*, len : LibC::SizeT, progress : ProgressFn, ud : Void*, out : WString*, error : Error*) : Status
  fun search = wopkg_search(c : Context, q : UInt8*, len : LibC::SizeT, limit : UInt32, out : WString*, error : Error*) : Status
  fun info = wopkg_info(c : Context, n : UInt8*, len : LibC::SizeT, out : WString*, error : Error*) : Status
  fun complete = wopkg_complete(c : Context, p : UInt8*, len : LibC::SizeT, limit : UInt32, out : WString*, error : Error*) : Status
  fun installed = wopkg_installed(c : Context, out : WString*, error : Error*) : Status
  fun fetch = wopkg_fetch(c : Context, req : UInt8*, len : LibC::SizeT, progress : ProgressFn, ud : Void*, out : WString*, error : Error*) : Status
  fun verify = wopkg_verify(c : Context, req : UInt8*, len : LibC::SizeT, out : WString*, error : Error*) : Status
  fun appimage_inspect = wopkg_appimage_inspect(c : Context, p : UInt8*, len : LibC::SizeT, out : WString*, error : Error*) : Status
  fun appimage_install = wopkg_appimage_install(c : Context, req : UInt8*, len : LibC::SizeT, progress : ProgressFn, ud : Void*, out : WString*, error : Error*) : Status
  fun appimage_remove = wopkg_appimage_remove(c : Context, n : UInt8*, len : LibC::SizeT, out : WString*, error : Error*) : Status
  fun appimage_update = wopkg_appimage_update(c : Context, req : UInt8*, len : LibC::SizeT, progress : ProgressFn, ud : Void*, out : WString*, error : Error*) : Status
  fun history = wopkg_history(c : Context, out : WString*, error : Error*) : Status
  fun history_record = wopkg_history_record(c : Context, e : UInt8*, len : LibC::SizeT, out : WString*, error : Error*) : Status
  fun rollback = wopkg_rollback(c : Context, id : UInt8*, len : LibC::SizeT, out : WString*, error : Error*) : Status
  fun system_call = wopkg_system_call(c : Context, req : UInt8*, len : LibC::SizeT, error : Error*) : Status
  fun release_plan = wopkg_release_plan(c : Context, req : UInt8*, len : LibC::SizeT, out : WString*, error : Error*) : Status
end
