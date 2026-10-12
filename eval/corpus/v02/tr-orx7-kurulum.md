# ORX-7 Sunucu Kurulum Kılavuzu

## Sistem Gereksinimleri

ORX-7 en az 8 GB bellek ve 4 çekirdekli bir işlemci gerektirir. Önerilen işletim sistemi Ubuntu 24.04'tür.

## Yapılandırma

Varsayılan bağlantı zaman aşımı 30 saniyedir ve `timeout_seconds` parametresi ile değiştirilebilir. Günlük dosyaları `/var/log/orx7/` dizinine yazılır.

## Hata Kodları

ORX-E104 hatası, lisans anahtarının süresinin dolduğunu gösterir. ORX-E221 hatası, veritabanı bağlantısının kurulamadığını belirtir.
