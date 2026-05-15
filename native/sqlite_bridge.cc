#include <napi.h>
#include <sqlite3.h>

#include <cmath>
#include <cstdint>
#include <limits>
#include <string>
#include <vector>

namespace {

class Statement : public Napi::ObjectWrap<Statement> {
 public:
  static Napi::FunctionReference constructor;

  static void Init(Napi::Env env) {
    Napi::Function fn = DefineClass(
        env,
        "Statement",
        {
            InstanceMethod("get", &Statement::Get),
            InstanceMethod("all", &Statement::All),
            InstanceMethod("run", &Statement::Run),
        });

    constructor = Napi::Persistent(fn);
    constructor.SuppressDestruct();
  }

  Statement(const Napi::CallbackInfo& info)
      : Napi::ObjectWrap<Statement>(info), db_handle_(nullptr), stmt_(nullptr) {
    Napi::Env env = info.Env();

    if (info.Length() != 2 || !info[0].IsExternal() || !info[1].IsString()) {
      Napi::TypeError::New(env, "Statement requires (dbRef, sql)").ThrowAsJavaScriptException();
      return;
    }

    db_handle_ = info[0].As<Napi::External<sqlite3>>().Data();
    if (!db_handle_) {
      Napi::Error::New(env, "Database handle is not available.").ThrowAsJavaScriptException();
      return;
    }

    const std::string sql = info[1].As<Napi::String>().Utf8Value();
    sqlite3_stmt* raw_stmt = nullptr;
    const int rc = sqlite3_prepare_v2(db_handle_, sql.c_str(), -1, &raw_stmt, nullptr);

    if (rc != SQLITE_OK) {
      Napi::Error::New(env, sqlite3_errmsg(db_handle_)).ThrowAsJavaScriptException();
      return;
    }

    stmt_ = raw_stmt;
  }

  ~Statement() override {
    if (stmt_) {
      sqlite3_finalize(stmt_);
      stmt_ = nullptr;
    }
  }

 private:
  static bool IsSafeInteger64(std::int64_t value) {
    return value >= static_cast<std::int64_t>(std::numeric_limits<std::int32_t>::min()) &&
           value <= static_cast<std::int64_t>(std::numeric_limits<std::int32_t>::max());
  }

  bool BindParameters(const Napi::CallbackInfo& info) {
    Napi::Env env = info.Env();

    if (!stmt_ || !db_handle_) {
      Napi::Error::New(env, "Statement has been finalized.").ThrowAsJavaScriptException();
      return false;
    }

    sqlite3_clear_bindings(stmt_);
    sqlite3_reset(stmt_);

    const int expected = sqlite3_bind_parameter_count(stmt_);
    if (info.Length() > static_cast<std::size_t>(expected)) {
      Napi::Error::New(env, "Too many bound parameters for statement.").ThrowAsJavaScriptException();
      return false;
    }

    for (std::size_t i = 0; i < info.Length(); i += 1) {
      const Napi::Value value = info[i];
      const int index = static_cast<int>(i + 1);
      int rc = SQLITE_OK;

      if (value.IsUndefined() || value.IsNull()) {
        rc = sqlite3_bind_null(stmt_, index);
      } else if (value.IsBoolean()) {
        rc = sqlite3_bind_int(stmt_, index, value.As<Napi::Boolean>().Value() ? 1 : 0);
      } else if (value.IsBigInt()) {
        bool lossless = false;
        const std::int64_t int_value = value.As<Napi::BigInt>().Int64Value(&lossless);
        if (!lossless) {
          Napi::RangeError::New(env, "BigInt value is out of int64 range.").ThrowAsJavaScriptException();
          return false;
        }
        rc = sqlite3_bind_int64(stmt_, index, int_value);
      } else if (value.IsNumber()) {
        const double num = value.As<Napi::Number>().DoubleValue();
        if (std::isfinite(num) && std::floor(num) == num &&
            num >= static_cast<double>(std::numeric_limits<std::int64_t>::min()) &&
            num <= static_cast<double>(std::numeric_limits<std::int64_t>::max())) {
          rc = sqlite3_bind_int64(stmt_, index, static_cast<std::int64_t>(num));
        } else {
          rc = sqlite3_bind_double(stmt_, index, num);
        }
      } else if (value.IsBuffer()) {
        const Napi::Buffer<std::uint8_t> buffer = value.As<Napi::Buffer<std::uint8_t>>();
        rc = sqlite3_bind_blob(stmt_, index, buffer.Data(), static_cast<int>(buffer.Length()), SQLITE_TRANSIENT);
      } else if (value.IsTypedArray()) {
        const Napi::Uint8Array arr = value.As<Napi::Uint8Array>();
        rc = sqlite3_bind_blob(stmt_, index, arr.Data(), static_cast<int>(arr.ByteLength()), SQLITE_TRANSIENT);
      } else {
        const std::string text = value.ToString().Utf8Value();
        rc = sqlite3_bind_text(stmt_, index, text.c_str(), static_cast<int>(text.size()), SQLITE_TRANSIENT);
      }

      if (rc != SQLITE_OK) {
        Napi::Error::New(env, sqlite3_errmsg(db_handle_)).ThrowAsJavaScriptException();
        return false;
      }
    }

    return true;
  }

  Napi::Value ReadCurrentRow(Napi::Env env) {
    const int column_count = sqlite3_column_count(stmt_);
    Napi::Object row = Napi::Object::New(env);

    for (int i = 0; i < column_count; i += 1) {
      const char* name = sqlite3_column_name(stmt_, i);
      const int type = sqlite3_column_type(stmt_, i);

      switch (type) {
        case SQLITE_INTEGER: {
          const std::int64_t value = sqlite3_column_int64(stmt_, i);
          if (IsSafeInteger64(value)) {
            row.Set(name, Napi::Number::New(env, static_cast<double>(value)));
          } else {
            row.Set(name, Napi::BigInt::New(env, value));
          }
          break;
        }
        case SQLITE_FLOAT:
          row.Set(name, Napi::Number::New(env, sqlite3_column_double(stmt_, i)));
          break;
        case SQLITE_TEXT: {
          const char* text = reinterpret_cast<const char*>(sqlite3_column_text(stmt_, i));
          row.Set(name, Napi::String::New(env, text ? text : ""));
          break;
        }
        case SQLITE_BLOB: {
          const std::size_t length = static_cast<std::size_t>(sqlite3_column_bytes(stmt_, i));
          const std::uint8_t* blob = static_cast<const std::uint8_t*>(sqlite3_column_blob(stmt_, i));
          if (!blob || length == 0) {
            row.Set(name, Napi::Buffer<std::uint8_t>::Copy(env, nullptr, 0));
          } else {
            row.Set(name, Napi::Buffer<std::uint8_t>::Copy(env, blob, length));
          }
          break;
        }
        case SQLITE_NULL:
        default:
          row.Set(name, env.Null());
          break;
      }
    }

    return row;
  }

  void ResetStatement() {
    if (stmt_) {
      sqlite3_reset(stmt_);
      sqlite3_clear_bindings(stmt_);
    }
  }

  Napi::Value Get(const Napi::CallbackInfo& info) {
    Napi::Env env = info.Env();

    if (!BindParameters(info)) {
      return env.Null();
    }

    const int rc = sqlite3_step(stmt_);
    if (rc == SQLITE_DONE) {
      ResetStatement();
      return env.Null();
    }

    if (rc != SQLITE_ROW) {
      std::string message = db_handle_ ? sqlite3_errmsg(db_handle_) : "SQLite step failed.";
      ResetStatement();
      Napi::Error::New(env, message).ThrowAsJavaScriptException();
      return env.Null();
    }

    Napi::Value result = ReadCurrentRow(env);
    ResetStatement();
    return result;
  }

  Napi::Value All(const Napi::CallbackInfo& info) {
    Napi::Env env = info.Env();

    if (!BindParameters(info)) {
      return Napi::Array::New(env);
    }

    std::vector<Napi::Value> rows;

    while (true) {
      const int rc = sqlite3_step(stmt_);
      if (rc == SQLITE_ROW) {
        rows.push_back(ReadCurrentRow(env));
        continue;
      }

      if (rc == SQLITE_DONE) {
        break;
      }

      std::string message = db_handle_ ? sqlite3_errmsg(db_handle_) : "SQLite step failed.";
      ResetStatement();
      Napi::Error::New(env, message).ThrowAsJavaScriptException();
      return Napi::Array::New(env);
    }

    ResetStatement();
    Napi::Array result = Napi::Array::New(env, rows.size());

    for (std::size_t i = 0; i < rows.size(); i += 1) {
      result.Set(i, rows[i]);
    }

    return result;
  }

  Napi::Value Run(const Napi::CallbackInfo& info) {
    Napi::Env env = info.Env();

    if (!BindParameters(info)) {
      return env.Null();
    }

    const int rc = sqlite3_step(stmt_);

    if (rc != SQLITE_DONE && rc != SQLITE_ROW) {
      std::string message = db_handle_ ? sqlite3_errmsg(db_handle_) : "SQLite step failed.";
      ResetStatement();
      Napi::Error::New(env, message).ThrowAsJavaScriptException();
      return env.Null();
    }

    Napi::Object metadata = Napi::Object::New(env);
    metadata.Set("changes", Napi::Number::New(env, static_cast<double>(sqlite3_changes(db_handle_))));
    metadata.Set("lastInsertRowid", Napi::Number::New(env, static_cast<double>(sqlite3_last_insert_rowid(db_handle_))));

    ResetStatement();
    return metadata;
  }

  sqlite3* db_handle_;
  sqlite3_stmt* stmt_;
};

Napi::FunctionReference Statement::constructor;

class Database : public Napi::ObjectWrap<Database> {
 public:
  static Napi::FunctionReference constructor;

  static void Init(Napi::Env env, Napi::Object exports) {
    Statement::Init(env);

    Napi::Function fn = DefineClass(
        env,
        "DatabaseSync",
        {
            InstanceMethod("exec", &Database::Exec),
            InstanceMethod("prepare", &Database::Prepare),
            InstanceMethod("close", &Database::Close),
        });

    constructor = Napi::Persistent(fn);
    constructor.SuppressDestruct();
    exports.Set("DatabaseSync", fn);
  }

  explicit Database(const Napi::CallbackInfo& info)
      : Napi::ObjectWrap<Database>(info), db_(nullptr) {
    Napi::Env env = info.Env();

    if (info.Length() < 1 || !info[0].IsString()) {
      Napi::TypeError::New(env, "DatabaseSync requires a database file path.").ThrowAsJavaScriptException();
      return;
    }

    const std::string db_path = info[0].As<Napi::String>().Utf8Value();
    sqlite3* raw_db = nullptr;

    const int rc = sqlite3_open_v2(
        db_path.c_str(),
        &raw_db,
        SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE,
        nullptr);

    if (rc != SQLITE_OK) {
      std::string message = raw_db ? sqlite3_errmsg(raw_db) : "Failed to open SQLite database.";
      if (raw_db) {
        sqlite3_close(raw_db);
      }
      Napi::Error::New(env, message).ThrowAsJavaScriptException();
      return;
    }

    db_ = raw_db;
    sqlite3_exec(db_, "PRAGMA foreign_keys = ON", nullptr, nullptr, nullptr);
    sqlite3_exec(db_, "PRAGMA journal_mode = WAL", nullptr, nullptr, nullptr);
  }

  ~Database() override {
    CloseInternal();
  }

  sqlite3* Handle() {
    return db_;
  }

 private:
  void CloseInternal() {
    if (db_) {
      sqlite3_close(db_);
      db_ = nullptr;
    }
  }

  Napi::Value Exec(const Napi::CallbackInfo& info) {
    Napi::Env env = info.Env();

    if (!db_) {
      Napi::Error::New(env, "Database is closed.").ThrowAsJavaScriptException();
      return env.Undefined();
    }

    if (info.Length() < 1 || !info[0].IsString()) {
      Napi::TypeError::New(env, "exec(sql) requires a SQL string.").ThrowAsJavaScriptException();
      return env.Undefined();
    }

    const std::string sql = info[0].As<Napi::String>().Utf8Value();
    char* error_message = nullptr;

    const int rc = sqlite3_exec(db_, sql.c_str(), nullptr, nullptr, &error_message);

    if (rc != SQLITE_OK) {
      const std::string message = error_message ? error_message : sqlite3_errmsg(db_);
      if (error_message) {
        sqlite3_free(error_message);
      }
      Napi::Error::New(env, message).ThrowAsJavaScriptException();
      return env.Undefined();
    }

    if (error_message) {
      sqlite3_free(error_message);
    }

    return env.Undefined();
  }

  Napi::Value Prepare(const Napi::CallbackInfo& info) {
    Napi::Env env = info.Env();

    if (!db_) {
      Napi::Error::New(env, "Database is closed.").ThrowAsJavaScriptException();
      return env.Null();
    }

    if (info.Length() < 1 || !info[0].IsString()) {
      Napi::TypeError::New(env, "prepare(sql) requires a SQL string.").ThrowAsJavaScriptException();
      return env.Null();
    }

    Napi::Object stmt = Statement::constructor.New({Napi::External<sqlite3>::New(env, db_), info[0]});
    return stmt;
  }

  Napi::Value Close(const Napi::CallbackInfo& info) {
    CloseInternal();
    return info.Env().Undefined();
  }

  sqlite3* db_;
};

Napi::FunctionReference Database::constructor;

Napi::Object InitAll(Napi::Env env, Napi::Object exports) {
  Database::Init(env, exports);
  return exports;
}

}  // namespace

NODE_API_MODULE(doc_sqlite, InitAll)
