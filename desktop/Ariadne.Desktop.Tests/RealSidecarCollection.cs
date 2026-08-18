using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U181：起真实 sidecar 子进程的测试类必须互斥执行。
///
/// **为什么需要这个集合**：`SidecarAppStateIsolation` 的模块初始化器把整个测试进程的
/// app-state 钉在一个临时目录里（U142）。它防住了「写用户真实目录」，但**没有**防住
/// 「测试类之间互相踩」——16 个测试类共享的正是那**同一份** app-state：
/// `secrets.json`、`provider_catalog.json`、`recent_projects.json` 都在里面。
/// xUnit 默认**跨测试类并行**，于是这些类会并发读写同一批文件。
///
/// **具体的踩法**（不是推测，是复现出来的）：`ResolveSidecar()` 里那句
/// `Environment.SetEnvironmentVariable("ARIADNE_SECRET_MASTER_KEY", ...)` 是**进程级**的，
/// 而每个类设的值都不一样（`first-run-master-key` / `settings-perf-master-key` / …）。
/// 后端 `LocalFileSecretStore::new` 在 sidecar **启动那一刻**读这个变量
/// （`core/src/config/secrets.rs:393`），因此子进程拿到的是「spawn 瞬间父进程里恰好是谁的值」。
/// 类 A 用 K1 加密写下 `secrets.json`，类 B 带着 K2 去 `set_secret`——
/// 而 `set_secret` 是读-改-写（`secrets.rs:598`，先 `read_values()` 再插入），
/// 于是 K2 去解 K1 的密文 ⇒ `aead::Error`。
///
/// **为什么这条比「偶尔红一次」严重**：报错文案是
/// `local secret encryption failed`，出现位置却在**读**路径（`GetProviderConfigAsync`）。
/// 我自己第一反应是去改 `core/src/config/secrets.rs`——
/// **测试基础设施的缺陷把人引向了产品代码**。
/// 而且随机红比稳定红危险：它训练人忽略红灯。
///
/// ⚠️ **串行化只消除并行交错这一半成因，另一半是磁盘残留**：
/// `secrets.json` 在整个测试进程生命周期里累积，串行后「类 A 用 K1 写、
/// 类 B 用 K2 读」依然会发生，只是变成稳定顺序而非随机。
/// 真正的收口在 <see cref="SidecarAppStateIsolation"/>：主密钥已改为**进程级唯一**，
/// 各类不再各设一把。两者缺一不可——串行防的是并发写坏文件，
/// 单一密钥防的是跨类解不开。详见该类型的注释。
/// </summary>
[CollectionDefinition("RealSidecar", DisableParallelization = true)]
public sealed class RealSidecarCollection
{
}
