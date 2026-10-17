# World Cleaner
эта программа может помочь очистить пустые регионы в плоском мире.

# Установка
Скачать бинарник из https://github.com/Azimkin/world-cleaner/releases и запустить. при первом запуске будет создан config.toml, Где нужно прописать путь к миру и выходной путь для удаленных регионов.

# Конфигурация
```toml
world_path = "./world" # путь к миру, который предстоит чистить
out_path = "./out" # путь к папке, в которую будут сохранены "пустые" регионы
block_whitelist = [ # вайтлист блоков в регионе, по ним определяется пустые они или нет.
    "minecraft:grass_block",
    "minecraft:dirt",
    "minecraft:bedrock",
    "minecraft:air"
]
threads = 0 # кол-во потоков, используемых для индексации. 0 - все.
flush_every = 512 # Раз в сколько регионов сохранять кэш и перемещать пустые.
```

# Метрики
В рамках эксперимернта была навайбкожена полная копия программы, но реализованная на incredibly slow python🤮 Разница приверно в 10 раз.

## Конфигурация
ОС: Ubuntu 24.04.4 
CPU: AMD Ryzen 9 9950X (32) @ 5.752GHz

## Результаты
Rust💪💪💪💪 (1 секунда)

![RUST](https://github.com/Azimkin/world-cleaner/raw/main/img/rust.png)

Poor python (10 световых лет)

![Python](https://github.com/Azimkin/world-cleaner/raw/main/img/python.png)


Сделано для проекта BTE 
