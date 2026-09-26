# indexer

给语料建倒排索引、做毫秒级本地检索，顺带找近似重复。
只吃 harvest 产出的 corpus.jsonl（每行一个 JSON 对象），全程本机计算，不联网。

实现：单个 C++17 源文件 tools/src/indexer.cpp，零第三方依赖（只用标准库 + Windows 下的 stdin/stdout 二进制模式）。

## 构建（首次使用前一次）

Windows（项目自带 mingw，g++ 16.2.0）：

    g++ -O2 -std=c++17 -Wall -Wextra -static -static-libgcc -static-libstdc++ modules/indexer/tools/src/indexer.cpp -o modules/indexer/build/indexer.exe

Linux：

    g++ -O2 -std=c++17 -Wall -Wextra -static-libgcc -static-libstdc++ modules/indexer/tools/src/indexer.cpp -o modules/indexer/build/indexer

macOS：

    g++ -O2 -std=c++17 -Wall -Wextra modules/indexer/tools/src/indexer.cpp -o modules/indexer/build/indexer

说明：

- modules/indexer/build/ 不存在时先建出来；构建应零警告。
- module.yaml 里的命令是 build/indexer build|query|dups；工具进程的 cwd 就是模块根目录 modules/indexer/。
- 编译器的运行库必须静态链进去：围栏只保证 PATH 上有系统自带的东西，不带编译器的 bin。少了上面的标志，
  Windows 上运行会报缺 libstdc++-6.dll / libgcc_s_seh-1.dll / libwinpthread-1.dll，Linux 上报缺 libstdc++.so.6；
  macOS 的 libc++ 属于系统组件，所以那行不需要标志。

## 协议

三个子命令都从 stdin 读一个 JSON 对象（UTF-8），结果写 stdout，失败写 stderr。

- argv[1] 是子命令；没有第二个参数 = 用法错。
- stdout 只放结果：UTF-8、紧凑，至多约 40 行；超出时最后一行如实写（已截断：还有 N 行未显示）。
- 失败一律 stderr 一行 + 非零退出码，不把失败写成空结果。

    退出码  含义
    2       用法或参数错（缺参数、空参数、超范围、未知子命令、stdin 不是 JSON 对象）
    3       文件打不开或写不了
    4       stdin 或 corpus.jsonl 的 JSON 非法
    5       index.bin 不是本模块的产物，或已损坏

## 分词规则

按 UTF-8 解出码点后：

- ASCII 字母数字连续段：转小写，长度 >= 2 才成 token（mod、crate、001、index 都算，a、x 不算）。
- CJK：落在下列区间的相邻两个码点组成一个二元组 token（如 模块、块系、系统）——
  扩展 A 3400-4DBF、基本区 4E00-9FFF、兼容表意 F900-FAFF、平假名+片假名 3040-30FF、谚文音节 AC00-D7AF。
- 单个汉字不成 token；查询中文请给两个以上连续字（如 检索）。非法 UTF-8 字节按单字节跳过，不当 token。

## index.bin 格式

全部整数为小端 u32，字符串为 u32 长度 + 原样 UTF-8 字节。

    偏移/段      内容
    magic        char[4] = "IDX1"
    docCount     u32：文档数
    termCount    u32：词项数
    docs 段      docCount 条，按 docId 从 0 递增：
                   u32 + bytes  rel（语料里的相对路径）
                   u32          chars（码点数；corpus 行里有 chars 字段就用它）
                   u32 + bytes  excerpt（正文前 160 个码点，原样，供检索输出摘要）
    terms 段     termCount 条，按词项字节升序排列：
                   u32 + bytes  term（UTF-8，ASCII 小写词或 CJK 二元组）
                   u32          postingCount
                   posting 每条：u32 docId + u32 tf（该词项在该篇出现次数），docId 升序

## build

    build/indexer build
    stdin: {"corpus":"<corpus.jsonl 路径>","out":"<index.bin 输出路径>"}

读 corpus.jsonl 的每一行（跳过空行，允许行尾 \r），取 rel / chars / text，分词建倒排索引并写出 out
（父目录不存在会自动建）。示例输出：

    已建索引：target/test-scratch/indexer-index.bin
    文档 5 篇，词项 98 个，文件 3191 字节

## query

    build/indexer query
    stdin: {"index":"<index.bin 路径>","q":"检索词","limit":10}

limit 为 1..50，省略 = 10；只读、可并发。查询词同样分词并去重，命中排序 = 命中 token 数降序，再按 rel 升序。
每行格式：命中token数 <TAB> rel <TAB> 摘要（摘要 = 正文前 160 码点，换行/制表压成空格）。示例输出：

    命中 3 篇（索引 5 篇，显示前 3 篇）；格式：命中token数 <TAB> rel <TAB> 摘要
    3	docs/alpha.md	Rust 的模块系统用 mod 关键字组织代码。一个 crate 可以有多个模块，模块之间通过路径互相引用。模块系统让大型项目保持清晰。
    3	docs/beta.md	Rust 的模块系统用 mod 关键字组织代码。一个 crate 可以有多个模块，模块之间通过路径互相引用。模块化让大型项目保持清晰。
    1	docs/delta.json	harvest 采集模块把文件抽成纯文本，输出 corpus.jsonl。

没命中会明说，例如英文查询打到中文语料：

    没有命中：索引里没有 q 的任何 token（索引文档 5 篇）。
    查询 token 共 3 个（中文需 2 字以上的连续片段才成 token）。

查询词分不出 token（比如只给一个汉字）时：

    查询没有可检索的 token（q 太短或全是标点）：检

命中多于 limit 时逐个 limit 输出，并注明还有更多命中未显示。

## dups

    build/indexer dups
    stdin: {"corpus":"<corpus.jsonl 路径>","threshold":0.6}

threshold 为 0..1，省略 = 0.6；只读、可并发。算法：

1. 每篇分词得到一个 token 序列；对连续 5 个 token 组成 shingle（NUL 分隔拼接）算 FNV-1a 64 哈希；
   不足 5 个 token 的篇，整篇 token 序列算一个 shingle；空篇没有 shingle。
2. 每篇保底保留 bottom-64：去重排序后取最小的 64 个哈希。
3. 两两估计 Jaccard 相似度：取两篇草图并集中最小的 64 个哈希组成 S，相似度 = |S 中两篇都有的哈希| / |S|。
   两篇 shingle 都少于 64 个时，这就是精确的 Jaccard。
4. 输出相似度 >= threshold 的文档对，按相似度降序（并列按 relA、relB 升序）；每行：相似度 <TAB> relA <TAB> relB，
   相似度保留 3 位小数。示例输出：

    近似重复 1 对（阈值 >= 0.600；语料 5 篇，每篇取 5-gram bottom-64 草图）；格式：相似度 <TAB> relA <TAB> relB
    0.717	docs/alpha.md	docs/beta.md

没有达到阈值的对时：

    近似重复 0 对（阈值 >= 0.990；语料 5 篇，每篇取 5-gram bottom-64 草图）；格式：相似度 <TAB> relA <TAB> relB
    没有达到阈值的近似重复。

## 错误示例（都走 stderr + 非零退出码）

    $ echo '{"corpus":"corpus.jsonl"}' | build/indexer build      # 退出码 2
    indexer: 缺少必填参数 out（字符串）

    $ echo '{}' | build/indexer query                              # 退出码 2
    indexer: 缺少必填参数 index（字符串）

    $ echo '{"index":"i.bin","q":"模块","limit":99}' | build/indexer query   # 退出码 2
    indexer: 参数 limit 超出范围 1..50

    $ echo '{"corpus":"nope.jsonl","out":"x.bin"}' | build/indexer build    # 退出码 3
    indexer: 打不开文件：nope.jsonl

    $ echo '{"corpus":"c.jsonl","out":"x.bin"' | build/indexer build        # 退出码 4
    indexer: JSON 非法：第 1 行（字节偏移 55）：对象的键必须是字符串

    $ build/indexer pollux                                          # 退出码 2
    indexer: 未知子命令：pollux（可用：build / query / dups）
